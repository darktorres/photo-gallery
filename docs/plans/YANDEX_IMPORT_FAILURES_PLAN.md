# Plan: thumbnail-cache setting, Yandex fixes, import-failure persistence

## Context

Five items. Findings come from reading the code and this machine's `D:\PhotoGallery\gallery.log` / catalog
(read-only) and from timing queries on a scratch copy of the catalog; the app was not run, so every "cause" has a
confirm step before its fix. Project rules: edit with Edit/Write, no new branch, native build, churn is not a concern
(pick the proper fix), and **use the designs already recorded in `docs/plans/` instead of parallel mechanisms**.

Decisions made with the user: the cache setting is a **live toggle**; the lost failures are **thumbnail /
placeholder failures**; persistence model = **classify by cause + retry as ledger state**.

> **BASE: this plan is rebased onto `main` at `1150d3b6`** (the per-drive lane refactor landed in `79e3010f`, `07b78c52`,
> `1150d3b6`; the working tree is clean). Every `file:line` below was re-read at that commit by three read-only agents;
> where a plan assumption turned out wrong it is corrected in place. What the refactor changed, for orientation:
>
> - The runner is **per-drive lanes with a coordinator**: pass bodies in `library/src/passes.rs`, the loop in
>   `lane.rs` (`Inner::run`, idle wait `lane.rs:42-47`), shared state in `inner.rs`/`shared.rs`, the lane registry and
>   probes in `coordinator.rs`/`lanes.rs`, **notice aggregation in `tally.rs`** (per-pass counts per lane, summed, with a
>   sorted `drives` list in `Notice::UnreadableFiles`).
> - The thumbs checkpoint is **per (height, volume)** (`thumbcache` `checkpoints` table, `Snapshot.volumes`,
>   `pending_on(volumes)`, `pending_sql(since, volumes) -> String`, `thumbs::run_while(.., volumes, ..)`).
> - An unplugged root gets **no lane** (`PROBE_DEADLINE` 5 s, `PLACE_TTL` 15 s in `coordinator.rs:40,42`; `Place` in
>   `lanes.rs`; `platform::drive::unplugged`), a returning drive re-plans lanes and wakes them, and a retired lane's
>   notices are settled away (`Tally::lane_retired`). In single-lane mode a lane's `VolumeSet` is still `All`.
> - `pipeline.rs`/`control.rs` gained `Budget`/governor/`cpu_permit`; no periodic tick exists in the sink.
> - **Nothing in the refactor adds retry, back-off, `next_try_at`, `io_attempts` or `idle_work`**; the lanes plan relies
>   on "the existing offline-file notice". Item 5 below is still to be built from scratch.
>
> **Three recorded, unimplemented plans already decide most of item 5** (`BACKGROUND_JOB_AUDIT_FIXES.md` "AUDIT",
> `DECODE_ONCE_PLAN.md` "DO", `BOUNDED_MEMORY_PLAN.md` "BM"): a typed `Unreadable(io::Error)`, an **`io_attempts` table**
> (not retry rows in `file_failures`), a three-attempt cap, `next_try_at` spacing, NotFound not counted, worklists skipping
> `next_try_at > unixepoch()`, the third attempt writing the terminal row. Section 5 is **reconciled with them** (see
> "Relationship to the recorded plans"). The audit text is itself stale in places (it says the next migration is `TO_10`;
> it is `TO_18`; its "runner deadline table" predates lanes), so it needs the amendments listed under 5.4.

This is revision 9: six adversarial reviews (eighteen reviewers), a reconciliation with the recorded plans, and a rebase
onto `1150d3b6`. "(review)" marks what a review changed; "(recorded)" marks a decision taken from a recorded plan;
"(rebase)" marks a change forced by the landed lane code.

**Two points to confirm with the user** (1 is **confirmed**; 2 was answered "something else" with no description, so it is
still open):

1. Placeholders get **no observer** (confirmed; not a churn choice): they have no checkpoint, so a discarded batch loses nothing
   durable, and their batch writes in one Immediate `catalog.tx()` held across up to 200 stores, which a second
   connection would collide with. Their failures stay recorded in that same transaction, they are routed through the same
   classifier and `io_attempts`, and they get their own notice (5.3).
2. No code path was found where a recorded failure is lost on a normal quit (see 5, "What was actually found"). If the
   symptom was the status bar's "N failed" vanishing after a restart, that count is `Running.failed` and thumbnail rows
   never reach the grid's failed filter or the status bar; no item here changes that. **Question: what exactly was seen?**

**Decisions settled with the user:**

- **D-A. Thumbs retry list: the due branch now (decided).** AUDIT E.2 says "no retry-list query: the `fold`/`first_failed`
  mechanism is deleted by BM D35/4.7". D35 (`BOUNDED_MEMORY_PLAN.md` D35, 4.4-4.6) is the unimplemented per-height cache
  state (cursors, laps, a `todo` set, a lease) that retires `checked`; it is not built and the landed lane refactor gave
  the checkpoint a different shape (per volume), so nothing removes `fold` yet. Decision: `first_failed` and `fold` are
  deleted now and Thumbs gets a `thumb_due` union branch (5.4); D35 later replaces the due branch with its laps.
- **D-B. Cap and spacing (decided):** the recorded **three counted attempts**, spacing 60 s then 10 min (parameters in
  `pg_model::retry`); NotFound 60 s and Offline 5 min, never terminal.
- **D-C. `FilePass` names (decided):** **`Duplicates`** and **`Placeholders`**.

**Builds** (the "one build at the end, no cargo between steps" rule conflicts with measure-first, which needs an
instrumented binary). **Build 1** = everything not gated by a measurement, plus the instrumentation (Yandex
reset/delegate/provider counters, `PG_PERF_LOG` phase split for stop latency, per-class/OS-code tally of failures). Run
it once, read the numbers. **Build 2** = only what the numbers decide: the B stop-check and shutdown grace, Yandex 2.5, the
circuit breaker. Within a build there is no cargo between steps. Every compile-gate (`just test`, `just lint` with its doc,
wasm, machete and boundaries checks) can spend a build, so the risky edits get a **hand review before build 1** (see
Verification).

**Implementation order.** Item 5 shares its change sets with AUDIT D (read errors), AUDIT C.5 (`Flusher`) and AUDIT B
(deadline table, rescans); implement the shared pieces **once, under whichever plan lands first**, not twice. This plan's
order: 1 -> 3, 4 -> 2 -> **5.0** (`Unreadable`, classifier) -> **5.3** (notices, new `FilePass`) -> **5.2** (observer on the
`Flusher` shape) -> **5.4** (`io_attempts`, views, worklist exclusions, wake) -> **5.5** (Import) -> 5.1 query fix -> 5.6
(breaker, per-root list). Measurement-gated parts of 5.1 and 2.5 are build 2.

---

## 1. Setting: enable thumbnail cache (live)

Today the cache is always on: `ThumbCache { store: Option<Store> }` (`crates/thumbcache/src/lib.rs:74`) is opened once
in `App::open` (`crates/app/src/app.rs:70-77`), leaked as `&'static`, shared by `Images`, the runner and writeback.
`ThumbCache::disabled()` (store `None`) already means "could not open"; this setting is a different thing.

**Design: a gate inside `ThumbCache`, not a re-open.** `suspended: AtomicBool` (default `false`) +
`set_suspended(&self)`/`suspended()` (not `enabled`, which collides with `disabled()`; the checkbox is "on" = not
suspended). When the open failed (`store: None`) the setting has nothing behind it; the `CacheDisabled` notice says so
(whether the checkbox should be disabled then: open item).

- Gated when suspended: `get_nearest_into` (-> `Ok(None)`), `put` (no-op), `put_batch` (-> `Ok(false)`).
- **Not gated:** `retag`, `forget`, `forget_height`, `pending`/checkpoint writes/`shrink`. The DB stays open and
  consistent, so re-enabling has no stale-row hazard (rows are keyed by mtime/rotation). `pending`/`pending_on`/`checked`
  need no gate (rebase, verified): a skipped pass never calls `checked` (`thumbs/src/lib.rs:234, 393-395`), so the
  per-(height, volume) `checkpoints` rows (`writer.rs:282-293, 393-395`) stay as they were, and the disabled-store form
  of `pending` already returns an empty list with `volumes` filled.
- Existing data is kept (no deletes, no growth). The CLI uses its own cache (`pg_library::open_thumbs`) and never reads
  prefs (`crates/cli` has no `Prefs` reference), so it is never suspended; `read_usage` ignores the gate (one doc line).
- Read from the code: writeback's `retag`/`forget` are not gated, `put` is fire-and-forget (no code treats a miss after
  `put` as an error), `Images::undecodable` still records Content failures while suspended, and a made thumbnail still
  clears its row. Cost while suspended: placeholders lose the `Read::Streamed` shortcut (`placeholders.rs:104`), so each
  reads its original (the tooltip says so); after a rotate the viewer re-decodes.

**Pref + UI** (the plumbing was read end to end; the touched-places list is complete). The Library panel has three
checkboxes of two kinds: `write_back` is saved only (read on demand, `app/src/edit.rs:28`);
`detect_borders`/`detect_duplicates` are saved and pushed to the job. (Real size is not a checkbox; it is the viewer's
`Z` key.)

- `prefs.rs`: `pub thumb_cache: bool` **after `serial_drives`** (rebase: the lane commit added that field last);
  `Default` (l.114-127, the only `Prefs` literal) `true`; add `"thumb_cache": true` at the end of `GOLDEN` (l.184-195) and
  of the inline JSON in `a_changed_value_is_written_as_its_own_text` (l.228; field order pinned). `#[serde(default)]`
  makes old files read as `true`. (`Prefs::features()` and `Features { borders, duplicates, serial_drives }` are
  unchanged by this item.)
- **No `Features` change** (the job-only prefs `detect_*` go through `Features`/`set_features`; this one also gates
  `Images` on the viewer path, which is not the job, so the gate lives on `ThumbCache`; the pref stays the single persisted
  source: written by the setter, read once in `App::open`; the atomic is only its live mirror).
  `settings_bridge.rs` (properties l.12-18, extern block l.22-31): `#[qproperty(bool, thumb_cache, cxx_name =
  "thumbCache", READ = read_thumb_cache, WRITE = set_thumb_cache_saved, NOTIFY, FINAL)]`, `fn read_thumb_cache(self:
  &Settings) -> bool;`, `fn set_thumb_cache_saved(self: Pin<&mut Settings>, value: bool);`. `settings.rs`, like the
  `detect_*` setters: `read_thumb_cache` returns `app::get().prefs().get().thumb_cache`; the setter, if changed:
  `change_prefs(|p| p.thumb_cache = value)`, `app::get().thumbs().set_suspended(!value)`,
  `app::get().library().wake_job()` (`app/src/library.rs:194` and `LibraryService` `:324`; a no-op if the library has not
  started, the first round reads `suspended`; otherwise `Runner::wake` (`runner.rs:120-122`) -> `Lanes::wake_all(true)`
  (`coordinator.rs:286-294`), which still calls `request_stop` on every lane before waking it: accepted (a stop is
  redundant when turning the cache off, and harmless when turning it on); the height/feature cells already use the
  non-stopping `wake_all(false)` (`runner.rs:72-82`) and AUDIT B.3 would make `wake()` do the same, which only improves
  this), then
  `self.thumb_cache_changed()`. The runner and `still_wanted` read `thumbs.suspended()` directly.
- `App`: add `pub fn thumbs(&self) -> &ThumbCache` beside `images()` (`app.rs:116`; the field is at l.40). In
  `App::open`, between `PrefStore::open` (l.78) and `LibraryService::new` (l.81):
  `thumbs.set_suspended(!prefs.get().thumb_cache)`.
- `LibraryPanel.qml` (required property `thumbCache`, signal `thumbCacheToggled(bool on)`, CheckBox + tooltip, like
  l.78-92); `Main.qml` l.651 `thumbCache: appSettings.thumbCache` and l.668 `onThumbCacheToggled: (on) =>
  { appSettings.thumbCache = on }`; `ui.ftl` `ui-thumb-cache`, `ui-thumb-cache-tooltip` beside l.72-73 (the lints only
  check parse, unique ids and the `ui-` prefix). The tooltip warns that with the cache off every thumbnail (and every
  placeholder) is decoded from its original each time. No pane-height bump (fixed 540 with a fill-height list); no QML
  stub changes (no LibraryPanel test); no doc lists settings. Notice strings live in `text.ftl`; there is one locale, `en`.

**Runner and thumbs pass** (rebase: pass bodies are `library/src/passes.rs`; the loop is `lane.rs`).

- `Thumbs::run` (`passes.rs:254-288`) already returns `Done` at once when `heights.get()` is `None` (l.257-259). Add: when
  `thumbs.suspended()`, skip `run_while` (before l.263) and call `files_read(FilePass::Thumbs, 0, true)` (precedent: the
  Borders skip, `passes.rs:226-229`). **The explicit call matters after the refactor:** the tally holds counts per lane per
  pass, and a lane that never runs a pass has no entry, so only an explicit `files_read(.., 0, true)` settles a stale
  `UnreadableFiles` (`Tally::settle_reads`, `tally.rs:71-85`); a stale `CacheFull` is resolved by `cache_room`
  (`tally.rs:114-119`) only for a lane that was recorded full, so the skip also calls `cx.inner.cache_room()`. It hides
  still-true unreadable files while suspended; they re-post after: accepted.
- **A flip while a pass runs** (no check before the call, it races): `put_batch` returns `Ok(false)` when suspended and
  `flush`'s existing `else` branch (`thumbs/src/lib.rs:337-339`, `if !stored { stats.cache_full = true }`) becomes
  `if cache.suspended() { stats.stopped = true } else { stats.cache_full = true }`. (Rebase, a correction of an earlier
  claim: a **height change** does **not** reach that branch: `flush` ends `Ok(stored && still_wanted())` (l.340), so a
  successful store followed by a false `still_wanted` stops the pipeline without setting `cache_full`, and
  `complete` is false because `tried < len`. There is no false-`CacheFull` bug to fix there; the early return at
  l.317-319 is `Ok(still_wanted())` and likewise never sets it.) `Ok(false)` already ends the pass (the writer stops and
  later batches are skipped, `pipeline.rs:411-440`); `put_batch` is all-or-nothing and buffers are recycled either way,
  so `made`/`kept` stay right. `complete` becomes `!cache_full && !stopped && tried == items.len()` (l.391) because
  `tried` is incremented per item (l.270), `put_batch` returns `Ok(false)` after the last item is counted, and a flip
  during the last batch would otherwise advance every volume's checkpoint over thumbnails that were never stored.
  `ThumbStats` gains `stopped` (one literal, l.221; the CLI reads only `cache_full`). `CacheFull` is posted only for
  `cache_full`. `still_wanted` (`&|| heights.get() == Some(height)`, the call at `passes.rs:263-271`) gets `&& !thumbs.suspended()`
  (shared borrows only). AUDIT E.2 also has `put_batch` `Err` record `cache_error` and end production without aborting the
  ledger flush: the same `flush` branch.
- Turning it **on**: `wake_job()` runs a round; `Thumbs` resumes from the checkpoint, which the suspended period never
  advanced; files changed meanwhile have `seq` above it (traced through off/quit/relaunch/on). Prerequisite:
  `heights.get()` is `Some` (existing).
- While suspended the Thumbs pass does not run, so Content failures are not discovered by it (placeholders and the
  on-demand path still record undecodable files). Acceptable.

**Tests:** thumbcache gate (get/put/put_batch no-ops; retag/forget still act; re-enable serves old rows; default not
suspended); prefs golden; runner (`runner.rs` tests, e.g. beside `a_full_cache_is_said_and_a_complete_pass_clears_it`,
l.415-430): suspended runs no pass, posts no `CacheFull`, settles a stale `CacheFull` and a stale `UnreadableFiles`;
**flip during the last batch leaves every volume's checkpoint unchanged** (a lane with two volumes); flip mid-pass posts
no `CacheFull`; a height change posts no `CacheFull` (pins today's behaviour).

---

## 2. Yandex screen keeps refreshing thumbnails

Pipeline: `Images::prefetch` (`app/src/yandex/images.rs:96`) fetches in 4 threads, each settle publishes a change; the
pump (`desktop/src/yandex.rs:~120`) coalesces and calls `poll()`; thumbs load as `image://yandex/<id>` (stable URL,
`cache` default true) in `ResultTile.qml:38-45`.

**Confirmed:** `drop_dead` (`desktop/src/yandex.rs:187-196`) resets the model per poll that reports dead results;
`QQuickRepeater` regenerates every delegate on reset (`qquickrepeater.cpp:573-575`); the log shows a steady stream of
dead results per search. **Not proven:** that this is the *visible* refresh: Qt destroys old delegates later
(`destroyObjectLater`) so their `Image`s keep the pixmaps referenced while the new ones request the same URLs (cache
hits; the 2 MB unreferenced limit is probably irrelevant here), and removal reflows the justified layout either way.

**Step 0 (build 1):** the fixes below and the counters ship together (there is no pre-fix baseline); the counters (model
resets, delegate creations, provider requests/decodes per search) are read after the fix and decide only 2.5. A remaining
visible refresh with zero resets means the cause is elsewhere (provider re-decodes).

**Re-entrancy rule** (corrected: today's re-entry writes, not only reads: `poll()` -> `set_busy` (l.275) emits
`busyChanged` -> `YandexPanel.qml:56-64` `onBusyChanged` -> `preview.open` -> the `Binding` -> `set_preview_item` ->
`refresh_preview`). cxx-qt does not guard it; what makes it survivable is: **never hold a Rust borrow of a field across a
signal-emitting call, and set fields before emitting** (as `set_preview_item` does, l.239-240). Every change below follows
it.

**Fixes:**

1. **Compute inside the reset, emit after.** `show()` resets, then `groups_changed()` sets the counts
   (`yandex.rs:146-151`), so on the first `found` the second Repeater has `otherCount == 0` and every tile requests its
   thumbnail until the counts flip, and `data()` reads stale or empty `frames` (`yandex.rs:285`). Callers of `relayout`:
   `groups_changed` (only `show` and `drop_dead` call it), `set_layout_width` (l.169), `set_gap` (l.179); each ends in
   `data_changed` for the four frame roles. Split: `compute_layout` (l.199-208: frames, `set_other_height`,
   `set_similar_height`), `emit_frames` (l.209-214), `relayout = compute + emit`; `set_group_counts`
   (`set_other_count`/`set_similar_count` from `results`). New `show`: `begin_reset_model; rust_mut().results = results;
   set_group_counts; compute_layout; end_reset_model; picks_changed` (`picks_changed`, l.218-233, emits `picked` over all
   rows, sets the counts and calls `refresh_preview`: harmless after a reset; the frame `data_changed` is redundant and
   dropped). `groups_changed = set_group_counts; relayout; picks_changed`. NOTIFYs emitted inside the reset only feed plain
   bindings (`Layout.preferredHeight`, `GroupHeader.count`); do not "set silently, emit later" (generated setters skip the
   notify when equal). `data()` during `end_reset_model` reads final fields: fine.
2. **`isOther` role:** `const ROLE_IS_OTHER: i32 = 264`; `data()` arm `i < self.results.first().len()`; `(ROLE_IS_OTHER,
   "isOther")` in `role_names()` (`yandex.rs:299-314`, names must match exactly: with `required property` a mismatch
   makes the delegate fail); `required property bool isOther` in `ResultTile.qml`; the Repeaters
   (`YandexPanel.qml:220,240`) use `visible: isOther` / `visible: !isOther`. No `data_changed` needed for it (a survivor
   never changes group). No keyboard navigation, cached selection index or `Flow` depends on `index < otherCount`; the
   only index users are `YandexPreview.item`, the one-shot `previewWhenFound` and click-time `tile.index`. No QML test or
   stub references `ResultTile`.
3. **Row removal instead of reset.** `beginRemoveRows` asserts `last < rowCount` (`qabstractitemmodel.cpp:3018`), so the
   state changes *between* begin and end. Bridge (copy `folders_bridge.rs:68-74`, next to `dataChanged`, l.96-103):
   `#[inherit] #[cxx_name = "beginRemoveRows"] fn begin_remove_rows(self: Pin<&mut YandexSearch>, parent: &QModelIndex,
   first: i32, last: i32);` and `endRemoveRows`; `QModelIndex::default()` is already used (l.210); borrow pattern as
   `gallery/model.rs:192-194`. Draft:

   ```rust
   // Picks
   pub fn remove(&mut self, r: Range<usize>) {
       let end = r.end.min(self.picked.len()); let r = r.start.min(end)..end;
       self.picked.drain(r.clone());
       self.anchor = match self.anchor {
           Some(a) if r.contains(&a) => None,
           Some(a) if a >= r.end => Some(a - r.len()),
           a => a };
   }
   // Results<T>
   pub fn remove_range(&mut self, r: Range<usize>) {
       let end = r.end.min(self.items.len()); let start = r.start.min(end);
       self.items.drain(start..end);
       self.first -= end.min(self.first) - start.min(self.first);   // right for a run spanning the boundary
       self.picks.remove(start..end);
   }
   ```

   `drop_dead` first computes `Vec<Range<usize>>` of dead runs (no `&` borrow of `results` may live across
   `end_remove_rows`), then per run, highest first: `begin_remove_rows(&root, a, b-1)`, `rust_mut().results.remove_range`,
   **`frames.drain(run)`** (`data()` indexes `frames[i]` by row, so survivors would read dead rows' frames until
   `relayout`), `end_remove_rows`; then one `groups_changed()`. `drop_where` today calls `picks.reset`, which clears the
   anchor; keeping it is an improvement; it stays `pub`. Fix the doc at `results.rs:60` (mentions a `retain` that does not
   exist and says `drop_where` is the only mutator). `show()` keeps the reset.
4. **Preview index after a removal: Rust adjusts it, once** (a QML `onRowsRemoved` handler runs N times inside the loop
   while Rust's `preview_item` is stale, so `refresh_preview` can fetch a shifted result's URL; QML's `preview.item` is
   the source of truth, the `Binding` at `YandexPanel.qml:272-276` only pushes it to Rust). **Capture the previewed hit's
   id before the removals** (guard `preview_item < 0` first: `row_of(-1)` is `None`, `rows.rs:5`); after the removals and
   before `groups_changed()`: `next = items().iter().position(|h| h.id == id).map_or(-1, qt_int)` (handles dead runs,
   group boundaries and clamping with no arithmetic; as a pure function over `(&[id], Option<id>)` for the test), then
   `self.as_mut().set_preview_item(next)` (no-op if equal; sets the field, emits, then `refresh_preview`). QML follows
   with one `Connections { target: yandex; function onPreviewItemChanged() { if (yandex.previewItem < 0) { if
   (preview.visible) preview.close() } else if (preview.item !== yandex.previewItem) preview.item = yandex.previewItem } }`
   (`previewItemChanged` is the generated NOTIFY, `yandex_bridge.rs:45`). Traced orders are safe: the `Binding` has no
   `when`/`restoreMode`, evaluates synchronously (`qqmlbind.cpp:785-811`) only when its own `value` changes, so a
   Rust-side write does not re-run it; the handler's `preview.item = new` makes it write the same value and
   `set_preview_item` returns early; `close()` sets `visible` first and the Binding writes -1 (equal); Escape closes via the
   same Binding path and the handler then sees `visible == false`. `close()` (`YandexPreview.qml:55-59`) restores panel
   focus via `onClosed` (`YandexPanel.qml:269`). In `poll()`, `drop_dead` runs before `set_busy` (l.265-275), so
   `previewWhenFound` indexes the post-drop results. Side effect: a dead previewed result closes the preview without the
   "cannot decode" flash (better). Do not use `Qt.callLater` (widens the wrong-URL window).
5. **Build 2, only if Step 0 shows re-decodes:** raise Qt's pixmap cache limit or cache decoded thumbnails in
   `yandex_picture` (`desktop/src/providers.rs:~100`). `Images::get` already returns `Err` at once for `Dead`
   (`images.rs:199`); what blocks a provider thread is `Fetching` (condvar, l.200-204); a `Queued` result is fetched
   inline on the pool thread (l.201-210); the 4-thread pool (`providers.cpp:128`) is shared by thumbnails and originals.
6. Move the doc comment above `drop_dead` (`yandex.rs:182-186`) to `relayout`.

Tests: `Results`/`Picks` remove-range (first/picks/anchor, a run spanning the group boundary, anchor above/inside,
clamping); the preview-index function. No proptests exist for either type.

---

## 3. Yandex full-screen preview goes black on prev/next

**Cause (read from `YandexPreview.qml:161-184`):** `full` is an async `Image` without `retainWhileLoading`; `step()` changes
the source, the pixmap is cleared, `sourceSize` -> 0 and `width/height` collapse to the empty `Theme.previewFill`;
neighbours are never loaded ahead.

**Fix** (mirror `ImageLayers.qml:62-76,102`, `Viewer.qml:395-405`):

1. `retainWhileLoading: true` on `full` (Qt 6.12, `qquickimage.cpp:988`). No `lastSize` latch: `sourceSize` stays the
   retained image's size until `requestFinished` swaps (`qquickimagebase.cpp:331-347`); A->B->C switches are safe. On
   `Error` the retained picture is cleared and "cannot decode" shows (accepted).
2. Neighbour prefetch: `previewPrevUrl`/`previewNextUrl` in `yandex/state.rs` + `yandex_bridge.rs`, set in
   `refresh_preview` **after** `preview_url` (`yandex.rs:256-259`) so `full` acquires the pixmap first:
   `original_url(results[(i ± 1) mod len].id)`, empty only when `len < 2` (dead results are dropped from `results` in the
   same `poll()` that reports them, so no dead handling is needed). Two hidden
   `Image { visible: false; asynchronous: true; cache: true }` with `sourceSize` unset (same cache key as `full`; they
   load while hidden). About three decoded originals in memory; no extra network. After a step the old current pixmap is
   unreferenced until `prev` takes it, so stepping back may re-decode; `retainWhileLoading` hides it; do not order for it.
   The neighbours share the 4 provider threads with grid thumbnails (2.5).

## 4. Yandex preview: mouse wheel = next/prev

Pattern: `Viewer.qml:299-315` (its accumulator is inline in the handler; `wheelBy` is `WheelListView.qml:37`, a ListView
smooth-scroll helper, unrelated: the notch function is **new**).

- **Decided with the user: the wheel always steps and no longer scrolls the image.** Today the wheel only scrolls the
  `Flickable` `previewView` (`YandexPreview.qml:148`), and only when the image is bigger than the view (1:1 or a huge
  image); the `MouseArea` at l.96-99 just swallows wheel events so the grid below does not move. A 1:1 image is still
  panned by dragging and by its two `ScrollBar`s (unchanged).
- **Placement: a `WheelHandler` declared as a child of `previewView`** (the Flickable), exactly as `WheelListView.qml:60`
  replaces a ListView's wheel scrolling. Traced in Qt: a handler declared inside a Flickable is reparented to its
  `contentItem` (`QQuickFlickablePrivate::addPointerHandler`, `qquickflickable.cpp:3015-3021`), items are visited
  children-first, and a handler gets the event before its item's own `wheelEvent`; `WheelHandler.blocking` is `true` by
  default (`qquickwheelhandler.cpp:323-329, 385-386`: it accepts the point), so the Flickable never scrolls. The content
  item is at least as large as the view (`contentWidth/Height = Math.max(width, full.width)`, l.156-157), so it covers the
  whole viewport in fit mode too. **No guard is needed** (the earlier "step only when the image fits" guard and its
  under-1-px tolerance are dropped). Keep the swallowing `MouseArea` (it still covers the preview bar, where the wheel
  should do nothing). Default `acceptedDevices` (mouse-sourced wheel events), as the Viewer's own handler
  (`Viewer.qml:302`); a device whose wheel events Qt flags as touchpad keeps its Flickable scroll, which matches the Viewer.
- Draft: `property real wheelNotches: 0`; `function notchesFor(angleY: real): int { wheelNotches += angleY / 120; const n
  = Math.trunc(wheelNotches); wheelNotches -= n; return n }`; in the handler `onWheel: (ev) => { const n =
  preview.notchesFor(ev.angleDelta.y); if (n !== 0) preview.step(-n) }`; reset `wheelNotches = 0` in `open()` (l.48).
- **Fix `step`** (`YandexPreview.qml:63`): `item = ((item + by) % total + total) % total` (`total = otherCount +
  similarCount`; handles multi-notch `by`).
- Test: QtTest's `mouseWheel` does not reach a `WheelHandler` (`WheelListView.qml:35`), so test `notchesFor` directly. No
  Yandex QML test infrastructure exists (no `Theme` singleton in `stubs/pg/gallery/qmldir`; `KeyMap` is stubbed): a
  `tst_yandex_preview.qml` needs the `Theme` stub.

---

## 5. Import-pipeline failures lost on exit (thumbs / placeholders / import)

### What was actually found

**No code path was found where a recorded failure is lost on a normal quit**, and nothing in the log shows one. What is
genuinely lost or hazardous: (a) the status bar's "N failed" (`Running.failed`) and everything never persisted by design
(unreadable files are count-only); (b) a job stuck in a blocking call at exit (the process ends when `main` returns, so
what it buffered is gone); (c) the exposure window: `Unwritten` up to 2 s / 200 entries, and the last `kept` (up to
~`2*size` results, `pipeline.rs:450`) which is handed to the writer only after the thread scope joins all 8 readers
(l.462), so a reader stuck in `CreateFileW` on a dead share holds back results that already arrived. The log (lines
1768-1774) shows slow stop, not lost failures: the cache had just been wiped (09:13:40), exit at 09:13:49, the error at
09:14:03; nothing was buffered. So this plan fixes persistence and shrinks the exit window, but the literal "lost on exit"
is only partly the cause. Thumbnail rows are visible in the inbox only (not in the grid's failed filter,
`Status::Failed` is `analysis.error IS NOT NULL`, `query/src/lib.rs:123-125`, nor the status bar).

The Thumbs ledger is `file_failures` in `catalog.sqlite`. Only `Panicked`/`Undecodable` are recorded (`retry = false`;
`thumbs/src/lib.rs:87-126`, `placeholders.rs:161-178`). `Unwritten` (`lib.rs:141-171`) buffers them, writes every
`BATCH` (= 200, l.31) or 2 s (`LEDGER_EVERY`, l.34) and again after `batched` returns (l.384, reached on a plain stop:
the sink returns `Ok` on `stop_requested`, `pipeline.rs:336`, the last `kept` is handed over with `wait = true`,
l.462-463, `ran?` is deferred to l.385). Only a *complete* pass advances the cache checkpoint (after the ledger write), so
anything unwritten is re-derived.

| | Candidate | Verdict |
|---|---|---|
| **A** | `Unwritten` lost on exit | Safe on a normal stop. Lost if the process ends while the job is stuck, or on a kill (re-derived). |
| **B** | Job outlives the 5 s stop | **Observed** (`gallery.log:1770, 1773`, 2026-11-12: `background job still running at exit`, then `round-failed: thumbnail cache closed` 11-14 s later). **Cause unproven:** `pending_in` (`thumbcache/src/reader.rs`, no `Control`) is the leading suspect; also `Catalog::open`, `recorded_failures`, the uninterruptible `pending_placeholders` (`placeholders.rs:79`), a reader blocked in a slow open on a dead share (`read_whole` opens before `pool.take`, `pipeline.rs:547-555`; stop is checked only between items, `control.rs:270`), or the pass being in Scans/Import. **HEAD differs from the log:** `Thumbs::run` now catches a pass error and posts `CacheNotStored` (`aeb7d2cb`, `b4515c7b`; its text is misleading for non-cache errors); Placeholders still yields `RoundFailed`. After the warning, `main` returns only after the synchronous `drop(engine)` and `app::shutdown` (`thumbs.close()` -> `writer.stop()`, a WAL merge of a 2.1 GB file); `shutdown` closes the cache under the running job (`app.rs:165`) because nothing connects `Library::stop`'s result to it (`desktop/lib.rs:212-214`). |
| **C** | Writer discards queued batches after it stops | **Confirmed**, narrower than first stated. After `flush` returns `Ok(false)` (cache full, `still_wanted` false, `lib.rs:336`) or errors, `batched` skips later batches (`pipeline.rs:411-425`) and drops the sink's last `kept` (l.462-464), failures included. Re-derived next pass because the checkpoint does not advance, so what is lost is the failure count shown and persistence on a stopped pass. Untested. A plain stop does not trigger it. |
| **D** | Placeholders | Failures share the batch transaction (`placeholders.rs:182-233`). No checkpoint: failures are re-derived from `pending_placeholders`; nothing durable is lost. |
| **E** | Never persisted by design | The status bar's "N failed" is `Running.failed`; the ledger holds only panicked/undecodable. Unreadable is count-only and folds `first_failed` (`lib.rs:286-292`: everything but `Panicked \| Undecodable` folds, so `Unreadable` and `Other` already hold the checkpoint back today). |
| **E2** | Import turns an offline drive into a permanent failure | **Confirmed by code:** any `Err` from the import pipeline goes to `writer.store_failure` (`import/lib.rs:157-194`, `writers.rs:477-495`), which writes `analysis.error` and clears `files.pending`, so an offline drive, a lock or access-denied is permanent (the inbox's `pass='import'` rows are a trigger mirror of `analysis.error`: `analysis_import`, `schema.rs:569-581`); only `import --retry`/`--again`, a file change or an import-version bump redo it. |
| **E3** | Hashes and Borders retry forever | Hashes leaves an unreadable file owed (`hashes_owed`, `hashes.rs:114-117`) and a content failure becomes sticky; one permanently owed file suppresses the whole catalog's "not a duplicate" list (`NOT_DUPLICATES`, `query/lib.rs:355-361`; `has_owed_hashes`). Borders leaves an unreadable file due, retried every round with no give-up (`borders/src/lib.rs:97-101`). |
| **F** | Native crash / power loss mid-pass | Loses `Unwritten`, re-derived. Both DBs are WAL `synchronous=NORMAL`, so on power loss the checkpoint could survive while the earlier ledger write does not (rare, low impact; out of scope). |
| **G** | Ledger rows deleted by design | `set_thumbs_version`/`set_borders_version`, `files_failures_stale`, `retry_failures`, `pair.rs:559`. Not exit-related. |

The catalog's `file_failures` is empty although the log shows `import-failed` (11-17) and `thumbs-failed` (file 179359);
that file's `mtime` has changed since, which fits the stale-row trigger. Not evidence of lost rows.

**Cite map (rebase; the findings above were read before the refactor, the lines moved):** `thumbs/src/lib.rs`:
`ThumbStats` literal 221, `Failed::of` 105-116 (the `io::Error` downcast 113), `recorded()` 121-126, `fold` 135-137,
`Unwritten` 141-171, `Writing` 175-182, `flush` closure from 266 (`tried += 1` 270, `is_current` 278, `fold` call sites 283
and 295, `put_batch` 322, `cache_full` branch 337-339, tail 340), `batched` call 343-385 (`Batch` 382), final ledger write
388, `complete` 391, `checked` 393-395, `shrink` 396, `Catalog::open(catalog_path, PATH_RULES)` 242 and 461.
`thumbs/src/placeholders.rs`: `pending_placeholders` 80, `batched` 99, `Read::Streamed` 104-106, `is_unreadable` 164-173,
`is_recorded` 177-179, batch transaction 181-234. `jobs/src/pipeline.rs`: `run` 184, `run_timed` 207 (`recv` 335, sink
call 343, stop check 345, `received < sent` 351-352), `Read` 82-95 (`Streamed` 87), panic arm 258-264, `read_error`
536-541, `FileTooLarge` 550, `try_reserve_exact` 689, `batched` 381-391 (`ctl.add` 452, `kept.push` 453, batch-due check
457-458, last `hand_over` 471-473, writer thread 411-440), `Options` 30 (gained `budget`). `app/src/library.rs`:
`JOB_STOP_WAIT` 34, its use 177, `wake_job` 194/324; `app/src/app.rs`: `stop` 149-157, `shutdown` 161 (`thumbs.close()`
165); `desktop/src/lib.rs:212-214` unchanged. `catalog/src/work.rs`: `pending_import_sql(retry_errors, volumes)` 51-67,
`hashes_from` 81-94, `borders_from` 70-79, `OWED_PLACEHOLDERS` 139-147, `pending_borders` 462-480, `pending_hashes`
482-496, `pending_placeholders` 500-522, `has_owed_hashes` 566 (`NOT_DUPLICATES` is `query/src/lib.rs:354`). `ledger.rs`:
`record_failures` 161, `clear_failures` 189, `retry_failures` 203; `writers.rs`: `store_failure` 477-495, `store_borders`
707, `record_failures_on` 800-826; `lifecycle.rs`: `set_import_version` 73, `set_borders_version` 223,
`set_thumbs_version` 258. `model/src/notice.rs`: `FilePass` 375-390, `UnreadableFiles` (now with `drives`) 675-680,
`Part` 891-894, `Op::LedgerWrite` arms 786-788, `Source` 1138-1163, sample 1409, `each_part` 1444-1445, size pin 1509,
resolver-table test 1836. `import/src/lib.rs`: `ImportStats` 38-45, `batched` 94, `store` 157-194 (`store_failure` 178);
`import/src/hashes.rs`: `batched` 56, unreadable arm 114-117, `complete` 86; `borders/src/lib.rs`: unreadable arm 97-101,
`batched` 136, `complete` 158.

### Relationship to the recorded plans (what this plan adopts, changes, or defers)

| Topic | Recorded decision | In this plan |
|---|---|---|
| Typed read error | AUDIT D.1: `Unreadable(io::Error)` in `pg_platform`, created in `pipeline::read_error`, `pg_video::Unopened`, `analysis/analyze.rs:40-41,196` (`file_hash_of`); passes test `downcast_ref::<Unreadable>()`; add `analysis -> platform` to `tools/crate-layers.txt` | **Adopted** (5.0); the classifier hangs off it |
| Attempt state | AUDIT D.2 / DO 3.3, 5.6: **`io_attempts(file_id REFERENCES files ON DELETE CASCADE, pass, attempts, next_try_at INTEGER NOT NULL DEFAULT 0, PRIMARY KEY(file_id, pass))`** + the reset trigger on `files` (`mtime, size, rotation, missing`); cleared by the pass on any stored result and at the terminal row | **Adopted** (5.4), replacing this plan's earlier `retry = 1` rows in `file_failures`, `attempts`/`next_try` columns, quiet-first-failure trigger work and `thumb_blocked`-over-`file_failures` views |
| Cap and terminal | three counted attempts; the third writes the terminal row (`store_failure` for import, `retry:false` ledger row for borders/dups); `--retry-errors` or a file change clears it | **Adopted** (D-B) |
| NotFound | not an attempt; `next_try_at = now + 60 s`; wait for the rescan | **Adopted** (5.4); Offline codes get the same not-counted treatment |
| Folder present | attempts counted only while the file's folder is present: `folder_present` moves from writeback's private `Inner` to `Catalog`, memoised per folder per flush, checked **outside** write transactions (`is_dir` on a dead share blocks) | **Adopted as the tiebreak** for the counted class only, on the worker side (5.0); codes decide first (no probe on the common paths) |
| Worklists | skip `next_try_at > unixepoch()` | **Adopted** (5.4) |
| Wake | AUDIT B.2/B.7: one deadline table in the runner (replaces `BUSY_RETRY`); `Catalog::idle_work()` includes the earliest `next_try_at` | **Adapted** (rebase): the table predates lanes; there is now one idle wait **per lane** (`lane.rs:42-47`, `BUSY_RETRY` `lane.rs:26`), so the earliest `next_try_at` over **that lane's volumes** feeds it (5.4). `Catalog::idle_work()` (EXISTS probes) is not built and is not needed for this |
| Rescans | AUDIT B.7: rescan every root at start, a cheap idle check, a slow full rescan (6 h) | **Supersedes** this plan's "rescan for codes 2 and 3" |
| `Flusher` | AUDIT C.5: a `Flusher` in `jobs/pipeline.rs` beside `batched` wrapping `flush`, with a classifier closure; systemic errors keep the batch and retry, never bisect; thumbs flush writes the cache so no bisect | **5.2's observer is built on its shape** and lands with it |
| `first_failed`/fold | AUDIT E.2: deleted by BM D35/4.7 ("a failure is retried once per lap"); "no retry-list query" | **Open: decision D-A** |
| Other worklist exclusions | AUDIT F.1: exclude `file_writes.temp IS NOT NULL` from `pending_import_sql`, `pending_borders`, `pending_sql` | the same three SQL strings 5.4 edits: land together |
| `FilePass::Duplicates` | DO: ledger row `FilePass::Duplicates` for a decode error/panic; AUDIT lists it out of scope | **Decided (D-C):** `Duplicates` and `Placeholders` |
| Offline drives | MULTI_DRIVE_LANES_PLAN (landed): an offline root gets no lane; its files wait "under the existing offline-file notice"; no retry design | **Complementary:** lane absence covers a drive that is already unplugged; `io_attempts` covers what the probe cannot (a mid-round unplug before the next 15 s probe, network shares, unknown places, single-lane `VolumeSet::All`, locks, access-denied). It adds nothing to the lanes plan and removes nothing from it |
| `Unopened` | AUDIT D.1: `Unreadable` is created in `pg_video::Unopened` | **Replace** `pg_video::Unopened` (a newtype over `io::Error`, `video/src/unopened.rs`; `pg-video` already depends on `pg-platform`) by `pg_platform::Unreadable`; `render/src/maker.rs:55-61` (`MakeError::of_video`) downcasts `Unreadable` instead |
| Writeback's own queue | `writeback/src/queue.rs` has `MAX_ATTEMPTS`, `write_failed(job, msg, offline)` (offline = attempts +0) and `NEXT_READY` back-off | **Left alone** (its own queue table, not a worklist); only `folder_present` (`writeback/src/lib.rs:470-475`, private on `Inner`, one caller at 442, `self` unused) moves to `Catalog` and writeback calls it there |

### Step 0 (build 1 instrumentation; tests pin each path)

- Stop latency: `PG_PERF_LOG` on the 190k library after wiping `thumbs.sqlite`, cold cache, to see which setup phase and
  which shutdown step (`drop(engine)` vs `writer.stop()`) holds the process. **Do not implement the B stop-check until
  this names the phase (build 2).**
- A per-pass tally by class and OS error code in the log (uses `classify_raw`, which lands in 5.0; expect `Other` = 0).
- C: thumbs test with a full cache / `still_wanted` false and a panicking file in a batch the writer would discard
  (assert the ledger holds it). B: the `CacheNotStored` catch (`b4515c7b`) keeps passing. **Stop-wait test harness**:
  `Runner::stop(timeout)` and `service.stop_within(timeout)` (`jobs/src/service.rs:120`, tested at l.174-208) already take
  the timeout; only the app wrapper hardcodes the private `JOB_STOP_WAIT` (`app/src/library.rs:34`, used at 177). Test at the
  runner/lane level with a pass that blocks (`Runner::stop` `runner.rs:89` -> `Lanes::stop` `coordinator.rs:314-333`,
  one shared deadline for the coordinator, the cropper and every lane); make the app wrapper's wait injectable only for the
  shutdown-ordering test.
- **Trace the known flake first (lanes-plan ledger F1):** `thumbs` test `a_file_that_doesnt_decode_is_recorded_and_one_unread_is_not`
  (`thumbs/src/lib.rs:687`) fails 2 of 5 runs alone (`left: (2,2) right: (1,1)`, "a recorded failing file is listed
  again"), predating the lanes. This plan edits that exact test and the `Unwritten`/`fold` code it exercises, so find the
  cause before changing either (trace, do not guess).

### Fixes

**5.0 `Unreadable` and the classifier** (lands first; used by 5.2, 5.4, 5.5, the Step 0 tally).

- **`pg_platform::unreadable::Unreadable(io::Error)`** (recorded, AUDIT D.1), created where a read or open fails:
  `pipeline::read_error` (`pipeline.rs:536-541`), `analysis/analyze.rs` `file_hash_of` (l.39-41; its caller adds a
  `with_context`, l.~99-100, so the marker must survive `anyhow` context), and **in place of `pg_video::Unopened`**
  (`video/src/unopened.rs`, produced at `video/src/ffi.rs:95-96`, re-exported `video/src/lib.rs:12`, consumed by
  `render/src/maker.rs:55-61`): `Unopened` is the same newtype over `io::Error`, so it is replaced rather than wrapped.
  The passes replace their **four** copies of `downcast_ref::<io::Error>()` (`borders/src/lib.rs:97`,
  `import/src/hashes.rs:114`, `thumbs/src/lib.rs:113`, `thumbs/src/placeholders.rs:171`) with
  `downcast_ref::<Unreadable>()`: this fixes a real bug (a decoder's `io::Error`, e.g. a truncated JPEG's `UnexpectedEof`,
  is "unreadable" today; it is Content). The import's `store` (`import/src/lib.rs:157-194`) has no classification at all
  today (every `Err` is `store_failure`, l.178). `FileTooLarge` (`pipeline.rs:550`) is Content (not an `Unreadable`);
  `OutOfMemory` from `try_reserve_exact` (l.689) is an `Unreadable` of class Transient. Add `analysis -> platform` to
  `tools/crate-layers.txt` (the line is `analysis: codec metadata model video`; `video`, `library` and `thumbs` already
  have `platform`) and `Cargo.lock`.
- **`classify_raw(code) -> Class`** and `classify_io(&io::Error)` in a **new** `pg_platform::errors` module (no collision:
  the crate's modules are `args audio clock console dir dirs dll drive file memory message names native picker priority
  process sqlite testing volume`; `Class` is `pg_model::retry::Class`, `platform` depends on `model`). There is **no
  existing OS-error classifier to reuse** (rebase correction: `drive.rs` has only the private `unplugged_code`, l.54-68,
  which knows Windows code 21 and Linux 19/6, and the public `unplugged(&io::Error)`, l.49 = `ErrorKind::NotFound ||
  unplugged_code`); the new table absorbs `unplugged_code`, and `unplugged` becomes a thin caller
  (`matches!(classify_io(e), Offline | NotFound)`) so scan and the coordinator (`lanes.rs:179,183`) keep one table. The
  `cfg(windows)` part only maps `io::Error` to a code, so the table is testable on Linux CI (on Linux a missing drive and a
  missing file are both ENOENT; on other platforms `ErrorKind` only, falling back to `Transient`). Paths are opened as
  verbatim `\\?\` / `\\?\UNC\` (`file.rs:24-46`); raw codes survive `read_error` and the downcast.
  - `Offline` (drive or share gone; **not an attempt**): Windows 21, 53, 55, 59, 64, 67, 121, 1112, 1167, 1231, 1232.
  - `NotFound` (**not an attempt**; recorded): 2 and 3 (3 is ambiguous with a removed drive letter; both wait for a scan).
  - `Transient` (**counted**): 32/33 (sharing/lock violation; files open with `FILE_SHARE_READ|WRITE|DELETE`, so rare), 5
    (access denied), 995, 1117, 1005 (a bad sector on one file is per-file; a pure function cannot tell it from a vanished
    device, so it gets the capped retries), `OutOfMemory`, other I/O.
- **`folder_present`** (recorded): before a **counted** attempt is recorded, check the file's folder, memoised per folder
  per pass, **outside** any write transaction and **on the worker side** (never on the sink thread: `is_dir` on a dead
  share cannot be cancelled and would stall the pipeline and the stop); it moves from writeback's private `Inner`
  (`writeback/src/lib.rs:470-475`: it reads `catalog.file_folder(file_id)` (`catalog/src/lookup.rs:40`) and tests
  `disk_path(catalog.rules(), &p).is_dir()`, `Err(_) => true`; `self` is unused; the one caller is l.442) to `Catalog`,
  and writeback calls it there. A missing folder makes the attempt not counted (NotFound rule). It runs only on
  the rare counted path.
- **Policy** in `pg_model::retry` (pure, wasm-safe; catalog cannot depend on jobs): `MAX_ATTEMPTS = 3`, spacing 60 s then
  600 s for counted attempts, 60 s for NotFound, 300 s for Offline (D-B, decided; parameters, not literals). The catalog
  takes them as bound SQL parameters, as `writeback/src/queue.rs` binds its `MAX_ATTEMPTS` (there is no `max_attempts!`
  macro in the repo; `schema.rs` never needs the numbers, the upsert does).
- `Class` is `Content | Transient | Offline | NotFound`; `Content` is everything that is not an `Unreadable`: a panic,
  `Undecodable`, `FileTooLarge`, a decode error.
- Edits to existing tests: `a_failure_is_classified_by_its_type` (`thumbs/src/lib.rs:653-669`),
  `a_file_that_doesnt_decode...` (l.687-: it deletes `b.png`, code 2, and asserts `unreadable == 1`; still true: NotFound
  is `Unreadable`, only not counted as an attempt; **this is the F1 flake**, see Step 0), the stopped-pass test (l.543) and
  the pass-error test (l.626); `placeholders_follow...` (l.743-) is unaffected.
  `placeholders.rs:164-179` (`is_unreadable`: any plain `io::Error`/`MakeError::Unreadable`, counted, nothing recorded;
  `is_recorded`: panic or `Undecodable`, recorded with `retry: false`) is routed through `Unreadable`/`classify`.
  Tests: `classify_raw` over the whole table on Linux CI; the `Unreadable` marker survives `read_error`; a decoder
  `io::Error` classifies as Content; a held file (`pg_testkit::held`) for an image and a video classify `Unreadable`, a
  truncated JPEG does not (recorded).

**5.1 B (query fix in build 1; the rest after the measurement, build 2).**

- The independent query fix: `+f.volume` in the full-pass `pending_sql` (about 1,330 -> 240 ms warm, **measured on the old
  single-checkpoint form** with `ORDER BY f.volume`). The landed per-volume `pending_sql` (`thumbcache/src/reader.rs:187-198`)
  writes `WHERE f.volume IN (..) ... ORDER BY f.volume, ..` for the full pass and `f.seq > ?2 AND +f.volume IN (..) ...
  ORDER BY +f.volume, ..` for the incremental one (the unary `+` is what keeps `files_seq`; a plain `f.volume IN` picks
  `(volume, file_ref)` and walks every file of the lane; pinned by `reader.rs:146-179` and `thumbcache/src/lib.rs`
  ~1020, ~1080). **Re-measure the full form with `IN (..)` before changing it**: it may already differ from the 1,330 ms
  case, and `docs/reference/IO.md` ("Drives: the spike") and the lanes plan's ledger item R1 record that 2M-row timings are
  unmeasured. If `+f.volume` still wins, the sorter replaces an index-ordered stream over ~190k rows (`temp_store =
  MEMORY`); time was measured, **peak memory was not** (see `BOUNDED_MEMORY_PLAN`): measure it. Re-measure cold before
  deciding how much stop-check work remains.
- Give the phase the measurement names a stop check: for `pending_in`, a SQLite `progress_handler`/`interrupt` on the
  pass's own plain `Connection` with `Control` passed in; same for `pending_placeholders`; skip `cache.shrink()` when
  stopped. `Catalog::open` stays unstoppable. (A reader stuck in a slow open on a dead share is the 5.6 breaker's job.)
- Shutdown: connect `Library::stop`'s result to `shutdown` (a flag, or a second bounded wait) so the cache is not closed
  under a running job (`App::stop` `app.rs:149-157` returns the result; `desktop/src/app.rs:59-65` only logs it;
  `desktop/src/lib.rs:212-214` is `app::stop(); drop(engine); app::shutdown();`; `shutdown` `app.rs:161` closes the cache at
  165 unconditionally; with lanes, `Lanes::stop` has already given every lane one shared 5 s deadline). A long grace has a cost: the process lingers without a window and a relaunch overlaps while the old
  one holds `.live`/`.lock`; skipping `thumbs.close()` leaves the WAL unmerged (merged at the next open). Choose the grace
  and the skip rule from the measurement. Post `background job still running` while the inbox is open.

**5.2 C + the exit window: record thumbs failures where they are first seen (thumbs only), on the `Flusher` shape.** A
`salvage` callback does not work (a `FnMut` moved into the writer thread cannot also run after `join`; the unrecoverable
discard paths are a `flush` that errors after `batch.drain(..)` and a failed `hand_over`, `pipeline.rs:463`). AUDIT C.5 adds
a `Flusher` in `jobs/pipeline.rs` beside `batched` (a classifier closure; systemic errors keep the batch and retry with a
stop-aware backoff; closures consume the batch only after commit; passes take `&mut Stats`): **build the observer in the
same restructure, not beside it.**

- **`run_timed` takes one event callback** (the tick and stop events happen inside `run_timed`'s loop,
  `pipeline.rs:335-345`, which has a single `sink: FnMut(usize, Result<R>, usize) -> Result<bool>`, l.214; `batched`'s sink closure already
  captures `kept`, so a second observer closure cannot compile): `enum Event<'a, R> { Result(usize, &'a Result<R>), Tick,
  Stop }`. `run` (l.184) wraps its old sink in it; `run_timed` (l.207)'s signature grows by one parameter (it already
  carries `expect(too_many_arguments)` with a reason, l.206). `batched_observed(.., tick: Option<Duration>, observe)`
  passes it down; `batched` (l.381) is a wrapper with `None` and a no-op, so its 5 production callers (`borders:136`,
  `hashes:56`, `import/lib:94`, `placeholders:99`, `thumbs:343`) and 6 test callers (`pipeline.rs` 1363, 1389, 1420, 1444,
  1470, 1493) do not change (annotate `None::<Duration>`); `pipeline::run`'s direct callers (`cli/src/borders.rs:79`,
  `crop/src/lib.rs:124, 249`) are untouched. `Options` gained `budget: Option<Arc<Budget>>` (l.30) in the same file:
  keep the rebase mechanical.
- **`recv_timeout` keeps end-of-stream intact:** `results` is a crossbeam `Receiver` (`pipeline.rs:21`, `bounded::<Outcome<R>>`;
  `recv_timeout` is available, only tests use it today); `let Ok(outcome) = results.recv() else { break }` (l.335) treats
  any `Err` as "all workers finished", so with `recv_timeout(tick)` match `Err(Timeout)` -> run the tick and `continue`,
  `Err(Disconnected)` -> `break` (disconnect stays the only end-of-stream signal, so the `received < sent` check at
  l.351-352 is unchanged). With `tick = None` behaviour is unchanged. **A tick is needed (rebase, verified):** nothing in
  the new governor/`Budget` provides one; the sink still blocks in `results.recv()` indefinitely, including while a lane
  is paused (readers wait at `checkpoint_until`; the pool's only timed wait is the reader-side `BUDGET_WAIT` 50 ms), and
  `Batch.every` is evaluated only after a result arrives (l.457-458). The stop check (today only after a result, l.345) is
  also made in the `Timeout` arm, and the observer runs **before** the `?` at l.343 and on `!go_on`; on stop it flushes
  **unconditionally inside the loop, before `return Ok(())`** (the final write after `batched`, `lib.rs:388`, runs only
  after the scope join, which waits for a stuck reader). The observer keeps its own elapsed-time check on the result path
  (a steady trickle never times out, as `Unwritten.since` does today). Tick/stop calls run inside `timing.sink` so
  `sink_busy_pct` counts them. The observer runs before `kept.push((i, result))` (l.453), taking `&result`;
  `ctl.add(len, result.is_err())` (l.452) already borrows it.
  `items: &[T]` is shared (`T: Sync`) and never moved, so the observer reads `items[i]` (id, seq, size, source, path) on the
  caller thread; the sink is a plain `FnMut` on the caller thread (`pipeline.rs:225,334`): no `Send` bound.
- **Shrink the exit window:** flush on `Event::Stop`; tick every **250-500 ms** (Content failures are rare and a tiny
  `record_failures` transaction costs ~1-3 ms).
- **Placeholders have no observer** (see the point to confirm; they keep recording in their batch transaction).
- *Ownership split of `thumbs::run_while`:* a sink-side struct `Sink { unwritten, failed, unreadable, failures, ledger:
  LedgerOutcome, catalog2 }` is declared **before** the call and borrowed (`|e| sink.observe(e)`) so it survives for the
  final write. The writer-side `Writing` keeps `{catalog, thumbs, tried, stats{made, changed, cache_full, stopped, ledger}}`
  (it still needs `catalog` for `is_current`/`clear_failures`). `Unwritten::write` no longer takes `&mut Option<i64>`. After
  `batched`: `ledger = merge(sink, writer)` (`Failed` wins, then `Written`, else `Untouched`; the panic test `lib.rs:446-` asserts at 488-504 hold),
  and the sink counters are copied into the returned stats. The `Err` arm of `flush` keeps no failure bookkeeping. The
  buffer element stays `(LedgerFailure, seq)` so a failed write can still be handled per item. **`first_failed` and `fold`
  go away (D-A, decided: the due branch)**: `Unwritten::write` loses its `first_failed` parameter, `fold` (l.135-137) and
  its call sites (l.164, 283, 295) are deleted, and `ThumbCache::checked(&snapshot, first_failed)`
  (`thumbcache/src/lib.rs:289`: `through = first_failed.map_or(snapshot.seq, ..)`; `writer.rs` `Op::Checked`) becomes
  `checked(&snapshot)`, which writes `snapshot.seq` to every one of the lane's volumes on a complete pass. A failed
  ledger write is then not silently dropped: it sets `LedgerOutcome::Failed` and **blocks the checkpoint** the way
  `complete` does (the pass is not complete), since otherwise a never-written failure would be skipped forever.
- *What the sink records per `Unreadable`/Content result:* Content -> the existing ledger row (`retry = false`);
  `Unreadable` -> one `io_attempts` call (5.4) with the class from 5.0 (counted for Transient after `folder_present`, not
  counted for Offline/NotFound), and at the cap the terminal ledger row (`retry = false`, "gave up after 3 attempts: ...").
- *Second connection:* `run_while` already has `catalog_path: &Path` and opens its own `Catalog` (the writer's); the sink's
  is new: `Catalog::open(catalog_path, pg_platform::native::PATH_RULES)` (`PATH_RULES` is not imported bare; l.242, 461
  spell the path), stays on the caller thread. Read-write secondary connections are routine (`app/inbox.rs:841`,
  `app/reader.rs:171`). Waiting for the write lock: `Catalog::set_busy_timeout(&self, Duration)` (`core.rs:206`) short
  on it (reset before the final write), **or, new since the refactor, `pg_catalog::wait_for_writers(Patience { stop, until
  })`** (`catalog/src/transaction.rs:46-72`, exported `lib.rs:50`, currently unused outside its tests): a thread-local,
  guard-scoped patience under which `write_transaction` retries `BEGIN IMMEDIATE` every 50 ms past the busy timeout until
  `stop` is set or `until` passes. The sink runs on the caller thread, so it can hold the guard around its writes with
  `stop` tied to the pass's `Control` (check that `Control` can hand out an `Arc<AtomicBool>`, else a small adapter): a
  stop-aware wait beats a fixed short timeout, and it is what the landed code provides for exactly this. Contention is low (the job's passes are sequential; the `put_batch` transaction is on `thumbs.sqlite`; other
  catalog writers are single short statements; lanes partition files by volume); the main case is a concurrent `gallery
  import` or `gallery thumbs` (~0.1-0.5 s Immediate transactions, estimated, not measured).
- *Errors:* `Unwritten::write` clears `failures` and posts `LedgerNotWritten` on any error today. A busy error keeps the
  failures buffered and retries on the next event or at the end; only a final failure sets `Failed` (which blocks the
  checkpoint, above). The typed `pg_catalog::Error` is converted to anyhow at l.166, so the `is_busy` check (`error.rs:64`, `pub`,
  re-exported at `lib.rs:45`) must come before the conversion. Wrap the observer in `panics::guard` or accept that a panic
  unwinds out of `thread::scope`.
- *Counts:* `ctl.add(len, is_err)` feeds `Running.failed` and `stats.failed` is separate: no double counting.
- Nothing can be discarded before it is observed. A re-record is an upsert on `UNIQUE(file_id, pass)`: no duplicates.
- **Interim until 5.4 lands:** 5.2 records Content only (as today) and keeps `first_failed`/`fold` exactly as
  `lib.rs:283-295` does for everything else; the deletion described above is part of 5.4, because only the due branch makes
  it safe (a retry must be re-listed by the query, not by the checkpoint staying back). Both land before build 1 anyway.

**5.3 D: notices** (rebased onto the tally). `Part` has one variant (`Code(&'static str)`, `notice.rs:891-894`); the
`UnreadableFiles` key is `(None, Part::Code(pass.code()))` with resolver `CompletePass(pass)` (`Source`, `notice.rs:1161-1163`),
`FilePass` has only Thumbs/Borders/Import (`notice.rs:375-390`), and any notice with that resolver is deleted by every
complete pass of that `pass` (`resolve_notices`, `catalog/src/inbox.rs:176-203`). **Who posts now:** passes call
`Inner::files_read(pass, unreadable, complete)` (`inner.rs:147-150`), which feeds `Tally::files_read(lane, drive, pass, ..)`
(`tally.rs:62-67`); `settle_reads` (`tally.rs:71-85`) sums the lanes' counts per `FilePass`, posts one
`Notice::UnreadableFiles { pass, count, drives }` (`drives` = the sorted lane labels, e.g. `E:`; the text builds " on E:"
from it, `client/src/text/notice.rs:208-219`, `text.ftl:333-341`), and resolves `Trigger::Pass(pass)` only when no lane
has unreadable files and every lane that ran the pass is complete. Today `Hashes::run` posts under `FilePass::Import`
(`passes.rs:177`), `Import::run` (`passes.rs:130-149`) posts nothing, and `Hashes::run` returns before `files_read` when
duplicates is off (`passes.rs:159-161`). **Decision: new `FilePass` variants `Duplicates` (D-C, decided) and
`Placeholders`**, each with its own key/resolver, so no pass clobbers another's notice (the proper fix over dropping the
notice or combining counts); `tally.rs` is generic over `FilePass` (a `BTreeMap` key), so it needs no structural change,
but its unit tests (`tally.rs:195-279`) pin `FilePass::Import` strings. Touches `FilePass::ALL`/`code()` (l.382-390),
`Op::LedgerWrite` arms (`notice.rs:786-788`, exhaustive), `golden.jsonl` (the `unreadable-files` line stays byte-stable,
`drives` is `skip_serializing_if` empty), `resolver_table.golden` (one row per pass), `Source`, the size pin (l.1509),
`passes_of` (`desktop/src/notices.rs:173`, uses `..`), the notice test (l.619, already carries `drives`),
`cli/src/report.rs` (`report_pass(pass, unreadable, drives, ..)`), `text.ftl` (**the title is a select on `$pass`
with `*[import]` as the default: each new pass needs its own title branch or it reads as Import**), the sample (l.1409)
and `each_part` (l.1444-1445), the wasm check. **The `UnreadableFiles` count keeps its meaning: files not readable this
pass** (so the shared text stays true; only the terminal rows and this count are user-visible, attempts are not).
**A lane that never runs a pass has no tally entry, so every skip branch must call `files_read(pass, 0, true)`:**
duplicates off in `Hashes::run` (l.159-161, so `Duplicates` clears its own notice), the cache-off Thumbs skip (section 1),
and the existing Borders skip (l.226-229). `Import::run` starts calling `files_read(Import, ..)` (it carries a new
`unreadable`/`complete`, 5.5); Placeholders posts its own count.
**Shrunken churn item (rebase correction):** the old per-round delete-and-reinsert is gone. `Tally::cache_room`
(`tally.rs:114-119`) resolves `Trigger::Pass(Thumbs)` only on the transition out of cache-full for a lane that was
recorded full, and `Thumbs::run` (`passes.rs:279-285`) then calls `files_read` which re-posts `UnreadableFiles(Thumbs)`:
one delete+insert (resetting `read`/`dismissed`/the repeat count, `app/src/inbox.rs:298-306`) per recovery from a full
cache, because `CacheFull` and `UnreadableFiles(Thumbs)` share the resolver `CompletePass(Thumbs)`. Not worth a separate
step now; the proper fix, if wanted, is a resolver for `CacheFull` that does not match the unreadable key (check
`resolver_table.golden` first). Recorded as an open item.

**5.4 E: retry state in `io_attempts`, classify by cause** (its own change set; recorded design, with the additions below).

*Schema* (one migration at the next free number: **`TO_18`** (the latest is `TO_17_PLACEHOLDERS`, `schema.rs:526`; the
`MIGRATIONS` array is l.51-68; the audit's "next is `TO_10`" is stale; the baseline `schema.rs:705` is frozen, comments
only); if the audit's slice lands first it owns the table and this plan adds only what is below; the schema check compares
against baseline + migrations, so it matches by construction; test the migration on a pristine schema; `lifecycle.rs`
tests that pin version steps need the bump; `rebuild.rs` `CARRIED` (l.134-156, which already lists `placeholders` and
`folder_failures`) gets `io_attempts` after `files`; `convert.rs` copies it; the lane refactor added nothing to the
schema, so nothing there collides):

```sql
CREATE TABLE io_attempts(
    file_id     INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    pass        TEXT    NOT NULL,           -- 'thumbs' | 'placeholders' | 'borders' | 'import' | 'duplicates'
    attempts    INTEGER NOT NULL,           -- counted attempts (Offline/NotFound leave it unchanged)
    next_try_at INTEGER NOT NULL DEFAULT 0, -- wall clock; the worklists skip rows with next_try_at > unixepoch()
    PRIMARY KEY (file_id, pass)
);
CREATE INDEX io_attempts_due ON io_attempts(pass, next_try_at);
CREATE TRIGGER io_attempts_stale AFTER UPDATE OF mtime, size, rotation, missing ON files
WHEN old.mtime IS NOT new.mtime OR old.size IS NOT new.size OR old.rotation IS NOT new.rotation OR old.missing IS NOT new.missing
BEGIN DELETE FROM io_attempts WHERE file_id = new.id; END;
```

(`next_try_at` is the audit's amendment to DO's version; the reset trigger is DO 3.3's.) **Why this beats retry rows in
`file_failures`** (decided by the recorded plans, confirmed by the investigation): the inbox, `live_failures!`, serials,
the `file_failures_live` index and the Import trigger mirror (`analysis_import`, `hidden`, `files_failures_stale` skipping
Import) are untouched; attempts are invisible to the user (only terminal rows show, which was the goal of "quiet first
failure"); no retry rows exist, so `OWED_PLACEHOLDERS`/`pending_borders` exclusions and `set_thumbs_version` keep their
semantics; the reset trigger makes identity comparison unnecessary in the exclusions.

*Behaviour* (one `Catalog::io_attempt(pass, file, class, now)` in `writers.rs`, guarded like `record_failures_on`
(`f.mtime/size/rotation` match and `f.missing = 0`), returning `{ recorded, terminal }`):

- `Transient` (counted, after `folder_present`): `attempts + 1`; at `MAX_ATTEMPTS` the row is deleted and `terminal = true`,
  and the pass writes the terminal result (below); otherwise `next_try_at = now + 60` (first) or `+ 600` (second).
- `NotFound`: attempts unchanged, `next_try_at = now + 60`; never terminal; the stale trigger clears it when a scan marks the
  file missing or changes it.
- `Offline`: attempts unchanged, `next_try_at = now + 300`; never terminal; cleared likewise.
- Any stored result for the (file, pass) deletes its row (the pass does it in the same transaction, as `ledger.clear` does);
  `set_thumbs_version`, `set_borders_version` and `set_import_version` also delete their pass's rows; the user's Retry
  (`retry_failures`) deletes the pass's rows.
- **Terminal result per pass** (recorded): Thumbs/Placeholders -> ledger row `retry = false` ("gave up after 3 attempts:
  ..."); Borders -> `store_borders(None)` plus the `retry = false` row (as its Content path does); Duplicates -> the DO
  failure row; Import -> `store_failure` (the mirror then creates the inbox row). Import below the cap writes **nothing**
  (no analysis row, so `files.pending` stays 1).
- **Worklist exclusion**, one macro `backing_off!(pass)` in `sql.rs` beside `live_failures!`: `EXISTS (SELECT 1 FROM
  io_attempts w WHERE w.file_id = f.id AND w.pass = <pass> AND w.next_try_at > unixepoch())`, guarded by an uncorrelated
  "any waiting row for this pass" check (O(1) on `io_attempts_due`) so the empty case is free on a 190k first import; added
  to `pending_files!` (`sql.rs:132-142`, used by `pending_import_sql(retry_errors, volumes)`, `work.rs:51-67`: the
  macro expands before the `{and}` volume filter, so the new predicate goes inside the macro), the duplicates/hashes
  worklist (`hashes_from`, `work.rs:81-94`; `pending_hashes` l.482-496), `pending_borders` (l.462-480, `borders_from`
  l.70-79), `OWED_PLACEHOLDERS` (l.139-147) and the thumbs `pending_sql`. A flag pass needs no due branch (its
  `pending`/`hashes_owed`/`borders_due` flag is still set, so a file whose `next_try_at` has passed is simply listed
  again). **Join order must survive** (rebase): `docs/reference/IO.md` ("Drives: the spike") records that borders and hashes
  drive from `files` through a `CROSS JOIN`, and the work-list tests pin it (`work.rs:654-672` `borders_statement`,
  `hashes_statement`; `work.rs:694-747`): the `EXISTS` must not change the plan. Re-check the `files_to_import` plan pins
  (`lifecycle.rs:1237-1243` with `VolumeSet::All`, `work.rs:694-698` with `VolumeSet::Only`: `files_to_import (volume=?)`,
  no `TEMP B-TREE`). The `retry_errors` UNION branch (`work.rs:56-63`) overrides the back-off (an explicit `--retry`).
- **Thumbs views** (the thumbcache reads the catalog "through views alone", `thumbcache/Cargo.toml:12`, `schema.rs:185`, and
  does not depend on `catalog`; today's three views are `thumb_sources` (`schema.rs:191-206` v6, original l.844-851: it
  already filters `f.missing = 0` and requires a clean `analysis` row), `thumb_clock` (l.854) and `thumb_folders` (v6)):
  `thumb_backoff(file_id)` = `io_attempts` for `pass = 'thumbs'` with `next_try_at > unixepoch()`, and (D-A, decided)
  `thumb_due(file_id)` = `pass = 'thumbs'` and `next_try_at <= unixepoch()`, both created in the `TO_18` migration. Update
  the list at `thumbcache/Cargo.toml:12` and the view-contract tests / `EXPLAIN QUERY PLAN` pins at
  `thumbcache/src/lib.rs` (~1020, ~1080) and `reader.rs:146-179`.
- **Thumbs `pending_sql`** (landed shape, `thumbcache/src/reader.rs:187-198`: `pending_sql(since_checked, volumes) ->
  String`, columns 0-8 `id, folder_id, name, mtime, rotation, poster_ms, seq, folder, size` read positionally at
  `reader.rs:61-72`, `LEFT JOIN entries t`, `ORDER BY {volume}, f.file_ref & 0xFFFFFFFFFFFF`): the changed branch gets
  `AND CASE WHEN <changed> THEN NOT EXISTS (SELECT 1 FROM cat.thumb_backoff b WHERE b.file_id = f.id) END` (the `CASE`
  wrapper is required: 4x at height 220 on a mostly-sticky 200,000-row table); with the due branch (D-A, decided) the
  incremental form becomes `SELECT <the 9 columns> FROM (<changed, not backing off, f.seq > ?2, +f.volume IN (..)> UNION
  ALL <thumb_due joined to thumb_sources, same entries test, same volume list, AND f.seq <= ?2>) ORDER BY +vol, fref &
  mask` (disjoint ranges by construction, `seq > cp` versus `seq <= cp`; **explicit outer column list** matching the
  positional read, not `SELECT *`); the full form keeps `f.volume IN (..)` (and `+f.volume` per 5.1, if the re-measure
  says so). **Per-volume semantics (rebase):** `?2` is the **least** checkpoint over the lane's volumes
  (`checkpoint_since`, `reader.rs:81-94`: `None` if any volume has no row), so the due branch's `seq <= ?2` is relative to
  that minimum; and `checked` writes one `through` to every volume of the pass, so a lane's volumes advance together.
  **Re-measure through the views on the landed per-volume code** (the table below is against the raw table and the old
  single checkpoint) and validate the SQL in sqlite3 on the scratch catalog before the build.
- **Wake-up** (rebased; the audit's `Catalog::idle_work()` with an EXISTS probe per index is not built and not needed:
  the lane already runs a full round on any wake): the lane's idle wait is `lane.rs:42-47` in `Inner::run`:
  `let idle = (!self.busy().is_empty()).then_some(BUSY_RETRY); if waker.wait(idle) == Woke::Quit { return; }` (`BUSY_RETRY`
  5 s, `lane.rs:26`; `Waker::wait(Option<Duration>)` `jobs/src/service.rs:48-66`; `Woke::{Work, Idle, Quit}` l.32-39, and the
  lane treats `Work` and `Idle` the same: a timeout runs a round). New: `idle = min(busy retry, next_retry)` where
  `catalog.next_retry(&self.volume_set())` is `SELECT min(next_try_at) FROM io_attempts JOIN files ... WHERE next_try_at >
  unixepoch() AND <the lane's volumes, VolumeSet::and_in, catalog/src/types.rs:23-65>` run per pass code (a UNION ALL from
  the pass list so each seek uses `io_attempts_due`), strictly future so a due-but-unconsumed row never makes the wait zero
  (a still-failing file re-arms into the future), with a ~5 s floor and a cap. **Connection:** the lane keeps
  `catalog: Option<Catalog>` across rounds (`lane.rs:37-39`; `round()` takes it at 95-98 and puts it back at 116), so at
  the wait it is `Some` after any successful round and the query reuses it; it is `None` on the first iteration and after
  a failed or panicked round (cleared at l.84), where a short-lived `Catalog::open` is the fallback. A skipped round
  (`again`, l.40-42, 66) bypasses the wait. A skipped pass (borders off, cache suspended) leaves its rows, which wake the
  lane at most once per `next_try_at`. A returning drive already re-plans and wakes lanes (`coordinator.rs` ~430-440), so
  `Offline` rows need no separate trigger. A Content failure is a normal ledger row: nothing here touches it.
- **Folds** (D-A, decided): none; the checkpoint advances on a complete pass and `first_failed`/`fold` are deleted (the doc
  claims at `NOTIFICATIONS_DESIGN.md:296` already say panics are not held back; the code agrees, `thumbs/src/lib.rs:295`).
  `complete` additionally requires `ledger != Failed` so a failure that could not be written is never skipped.
  **Lane-ledger item L4** (thumbs checkpoint: A completes, B stops, redo only B) is open and untested: add the test here,
  since this change touches `checked` and its volumes.
- **Offline reporting:** through the existing `UnreadableFiles` count (5.3), which already names the drives. **Lane
  interaction (rebase):** an unplugged drive's lane is retired (<= ~15 s probe TTL + `RETIRE_WAIT` 10 s) and
  `Tally::lane_retired` (`tally.rs:141-159`) settles that lane's notices away, so a drive unplugged *before* a round never
  produces `io_attempts` rows (no lane, no reads) and shows only the lanes plan's offline-file notice; `Offline` rows arise
  from a mid-round unplug, shares/unknown places, single-lane `VolumeSet::All`, and they wait out their `next_try_at`.
- **E2 repair** (new, one-time Rust `convert` step, since SQL steps never reach rebuilt catalogs, `convert.rs:1-12`):
  `analysis.error` rows an offline drive or lock wrongly made permanent: match the `(os error N)` suffix after the `read:`
  context (verify the `{e:#}` text first); where `classify_raw(N)` is not Content, delete the analysis row
  (`analysis_removed` sets `pending = 1`). Optional but it repairs existing damage.

*Query measurements* (scratch copy of the 190k-file catalog, real `thumbs.sqlite`, read-only; synthetic ledgers; best of
2-5, warm; reproduced by three reviewers; SQLite 3.49 in Python vs 3.54 bundled by rusqlite; **old single-checkpoint
shape, to be re-measured**):

| query (`pending_in`) | height 220 (all thumbed) | height 440 (none thumbed) |
|---|---|---|
| full pass today (`ORDER BY f.volume`: scans `sqlite_autoindex_files_1`) | 1,330 ms | 1,870 ms |
| `ORDER BY +f.volume` | **222-240 ms** | **786-800 ms** |
| `+volume`, plain `NOT EXISTS` on a 200,000-row mostly-sticky table | **872 ms** | 1,000 ms |
| same with the `CASE WHEN <changed> THEN NOT EXISTS (..) END` wrapper | **229 ms** | 1,100 ms |
| incremental, 50 changed files, blocked + due `UNION ALL` (200,000 rows, 3,750 due) | 65 ms | 67 ms |
| incremental held back to the middle (what the `fold` design costs) | ~310 ms | ~450-1,020 ms |

The views flatten; `SEARCH f USING INDEX files_seq` survives in the compound query; the due branch needs its partial/
covering index (without it 309 ms at 200,000 rows, with it 0.1-1.5 ms: **`io_attempts_due` is required**); `unixepoch()`
inside views does not block index use and is constant per statement; the due branch costs ~60 ms with 3,750 due rows even
when nothing is pending. Hash-load and peak-memory of the `+volume` sorter: measure.

*Edit (not just add):* docs `catalog/src/ledger.rs:24, 38, 46-51, 83-86, 149-150`, `schema.rs` comments,
`thumbcache/Cargo.toml:12`, `thumbcache/src/lib.rs` (checkpoint doc), `NOTIFICATIONS_DESIGN.md:296,351,357`; the recorded
plans' own amendments (AUDIT "Docs to amend", DO 3.3/5.6: add `next_try_at`, `classify`, the typed `Unreadable`; AUDIT
D.2 migration number `TO_10` -> `TO_18`; AUDIT B.2/B.7 "one deadline table in the runner" -> per-lane idle wait,
`idle_work()` dropped; AUDIT E.2 / BM 4.7 `first_failed` is deleted by this plan's due branch, so D35 later replaces the due
branch, not `fold`); `docs/plans/MULTI_DRIVE_LANES_PLAN.md` (a line on how `io_attempts` complements "waits under the
offline-file notice") and `docs/reference/STORAGE.md`/`IO.md` where they describe offline files; tests
`runner.rs:394-411` `an_offline_file_is_said_not_recorded_and_a_complete_pass_clears_it` (it posts
`UnreadableFiles { pass: Thumbs, count: 1, drives: [] }`), `runner.rs:336-351` (cache error), `:415-430`, `:521-532`,
`thumbs/src/lib.rs:446-504, 543, 626, 653-669, 687-`, `library/tests/borders.rs:91` (an unreadable file stays due: now backs
off), `stop_at_batch.rs:74-83`, `work.rs:654-672, 694-747`, `writers.rs` tests from l.1172 (the borders-after-import tests,
which call `pending_borders(&VolumeSet::All)`), the `b4515c7b` cache-error test, `tally.rs:195-279`, the `EXPLAIN QUERY
PLAN` pins.
*Reconcile first* with BM `first_failed` removal (`BOUNDED_MEMORY_PLAN.md:1261`), `GUI_THREAD_PLAN.md:784`.
*Lint/doc rules* (`tools/lint.py:25-28`): `cargo doc --workspace --no-deps --document-private-items` with `-D warnings`
(renaming/removing `Failed`, `first_failed`, `fold`, `Unwritten::write` breaks intra-doc links: grep first);
`cargo check -p pg-model -p pg-client --target wasm32-unknown-unknown`; `cargo machete`; `tools/boundaries.py` against
`tools/crate-layers.txt` (`analysis -> platform` is a recorded addition).
*Tests to add:* `io_attempts` upsert per class (counted 1, 2, 3rd terminal; NotFound/Offline never terminal and never
counted; a duplicate in one batch; a stale-identity file writes nothing); the three resets (stored result, file change/
missing trigger, version bumps, Retry); worklist exclusion per pass (`backing_off!` leaves a due file listed and a waiting
file out; the empty case is free; `files_to_import` plan pin); idle wait never spins (skipped pass, current entry, missing)
and `next_retry` respects the lane's volumes; terminal result per pass (Thumbs ledger row, Borders `store_borders(None)` +
row, Import `store_failure`, Duplicates row); Import below the cap writes nothing; `folder_present` memoised, called outside
a write transaction and not on the sink thread; observer ordering (a failure observed while the writer has stopped; the tick
fires with no results; flush on stop; a busy write keeps the buffer and retries); `recv_timeout` end-of-stream; the
notice is not clobbered and not deleted-and-reinserted each complete pass; E2 repair (a `read:` EACCES row is re-queued, a
corrupt-file row is not).

**5.5 Import pass (before 5.1; recorded in AUDIT D.2).** `import::store` (`import/lib.rs:157-194`) stops sending every `Err`
to `store_failure` (l.178): an `Unreadable` goes through `io_attempts` (5.4) and leaves the file pending below the cap
(`pending` stays 1: no analysis row is written); Content still goes to `store_failure` (sticky, `analysis.error`, mirror
row, grid filter, `--retry`). `ImportStats` (`import/lib.rs:38-45`: `pending, files, failed, changed`) gains `unreadable`
and a new `complete` (`complete = files + unreadable + changed == pending`; the `continue` paths skip `files += 1`; the
empty-items early return at ~l.84 gives `complete: true`), `Counts` (l.48-52) and the literal follow; `Import::run`
(`passes.rs:130-149`, today it calls `imported()` only when `s.files > 0`) calls `files_read(Import, ..)` (5.3). The import's
video path returns a `pg_video::Unopened` today and `store` does no classification at all; with `Unreadable` replacing
`Unopened` (5.0) one check covers images and videos. **Cost note:** a pending file is relisted every round (no
checkpoint); `backing_off!` now bounds it to one open per `next_try_at`, which is why the exclusion is in `pending_files!`.
**Interim until 5.4 lands:** `Unreadable` keeps going to `store_failure` as today; never leave Import in a "pending with no
give-up" state between builds. **Hashes/Duplicates:** the same `io_attempts` path (the file stays owed with a back-off and
at the cap becomes the DO terminal row, which also closes the `NOT_DUPLICATES` suppression); a content failure stays a
sticky `analysis.error` (DO: a ledger row, not `store_failure`, once the Dups pass exists).

**5.6 Remaining item-5 change sets (in scope; after 5.4; re-evaluated against the coordinator and the recorded plans).**

- **Per-root circuit breaker** (bounds N slow failed opens on a dead share the first time, which `io_attempts` spacing does
  not; also the stuck-reader stop latency); build 2, after the Step 0 numbers. **Rebase: lower value than before.** A dead
  share's reader now holds only **its own lane** (one lane per drive; other drives keep working), so the breaker no
  longer protects the rest of the library; what remains is the first-time cost of N slow failed opens inside that lane and
  the stop latency (`Lanes::stop` joins every lane against one shared 5 s deadline). Build it only if the build-1 numbers
  show the dead-share case matters. Design (review): the read closure cannot
  return an error and never sees the open result (`Read` is `File | Streamed | Gather`, `pipeline.rs:82-95`), so it needs a
  new `Read::Fail(io::Error)` handled like the panic arm (l.258-264: the stub must also call `pool.pass(i)` or the pool
  deadlocks); the trip is seen only at the observer (sink thread), so dead roots live in a `Mutex<BTreeSet>` read by the 8
  reader threads and it lags by the results in flight and by readers already blocked (it bounds the repeated cost, not the
  first timeout); videos are opened by FFmpeg in `process` (`lib.rs:362`, `import:113`, `hashes:73`), so the check must
  also be there; the root for an item comes from a per-pass `Vec<Option<root_idx>>` indexed by folder
  (`work.parts(item.path)[0]`; `Catalog::roots()` at `roots.rs:53-55`, `rules.is_under` at `model/src/rules.rs:281`;
  `covering_root` is `pub(crate)` at `catalog/src/types.rs:123` and per-call; call `roots()` before `catalog` moves into
  `Writing`; a lane's root set is its `VolumeSet`), shared across passes as a `RootGate`
  in `jobs`. Trip rules: immediately only on 53, 67, 1231, 1232 (host/share level); 59, 64, 21, 121 need 2+ failures on
  the root with zero successes since; never on 2, 3, 5, 32, 123, 206. A false trip is pass-scoped. Consider
  `CancelSynchronousIo` from the stop path. The coordinator's drive probe (`unplugged`) already keeps unplugged local
  roots out of lanes, so this matters for network shares, unknown places, mid-round unplugs and single-lane
  `VolumeSet::All`.
- **Per-root list in the `UnreadableFiles` notice: dropped, already built at drive granularity.** The lane commit added
  `drives: Vec<String>` (`#[serde(default, skip_serializing_if = "Vec::is_empty")]`, `notice.rs:675-680`; the lane's label,
  e.g. `E:`, from `drive_label`, `coordinator.rs:392-394`) and the text " on E:" (`client/src/text/notice.rs:208-219`). A root
  path list would be a second payload field for little gain.
- **Dropped as superseded:** the rescan for codes 2 and 3 (AUDIT B.7's startup/idle/slow rescans and `next_try_at` in
  `idle_work()` cover it; NotFound rows wait for them); the persisted per-root offline mark (`io_attempts` spacing, the
  coordinator's probe and lane absence bound the cost); the quiet-first-failure trigger work and the "will retry" label
  (attempts are not inbox rows).

---

## Open items to settle during implementation

- The two points to confirm (placeholders get no observer; what exactly was seen as "lost on exit").
- Whether `Control` can hand the sink an `Arc<AtomicBool>` for `wait_for_writers(Patience { stop, until })` (5.2), and the
  `until` bound.
- The trace of the F1 flake (`thumbs` test at `lib.rs:687`) before the `Unwritten`/`fold` rewrite.
- Whether `CacheFull` should get a resolver that does not clear `UnreadableFiles(Thumbs)` (5.3, optional).
- Which setup phase holds the stop (build-1 numbers) and the shutdown grace/skip rule; whether a slow open on a dead share
  is the cause of "still running at exit" (then the breaker moves up).
- Spacing values (60/600 s counted, 60 s NotFound, 300 s Offline) and whether Offline should ever go terminal.
- Whether the cache checkbox is disabled when the cache failed to open, and the `CacheDisabled` text.
- Peak memory of the `+f.volume` sorter on 190k rows; re-measure all query timings on the landed per-volume code.
- Whether the audit's migration slice (which would own `io_attempts`) lands before this change, and who owns `Flusher`.

## Critical files

- Cache setting: `crates/thumbcache/src/lib.rs`, `crates/app/src/{app,prefs,library}.rs`, `crates/library/src/{passes,runner,coordinator}.rs`,
  `crates/thumbs/src/lib.rs`, `crates/desktop/src/{settings,settings_bridge}.rs`,
  `crates/desktop/qml/{LibraryPanel,Main}.qml`, `crates/client/locales/en/ui.ftl`
- Yandex: `crates/desktop/src/{yandex,yandex_bridge}.rs`, `yandex/state.rs`, `crates/client/src/{results,picks}.rs`,
  `crates/desktop/qml/{YandexPanel,YandexPreview,ResultTile}.qml`
- Failures: `crates/platform/src/{unreadable,errors,drive}.rs` (new/changed), `crates/model/src/{retry.rs (new),notice.rs}`,
  `crates/jobs/src/{pipeline,service,control}.rs`, `crates/thumbs/src/{lib,placeholders}.rs`,
  `crates/thumbcache/src/{reader,lib,writer,schema}.rs`, `crates/library/src/{passes,lane,inner,tally,coordinator}.rs`,
  `crates/app/src/{app,library,images,reader,inbox}.rs`, `crates/desktop/src/{app,lib,notices}.rs`,
  `crates/catalog/src/{schema,ledger,lifecycle,rebuild,convert,writers,work,types,sql,inbox,core,error,transaction,lookup}.rs`,
  `crates/client/src/text/notice.rs`, `crates/client/locales/en/text.ftl`, `crates/cli/src/report.rs`,
  `crates/borders/src/lib.rs`, `crates/import/src/{lib,hashes}.rs`, `crates/analysis/src/analyze.rs`,
  `crates/video/src/{ffi,unopened,lib}.rs` (`Unopened` replaced by `Unreadable`), `crates/render/src/maker.rs`,
  `crates/writeback/src/lib.rs` (`folder_present` moves), `tools/crate-layers.txt`
- Recorded plans to amend: `docs/plans/BACKGROUND_JOB_AUDIT_FIXES.md`, `DECODE_ONCE_PLAN.md`, `BOUNDED_MEMORY_PLAN.md`,
  `docs/reference/NOTIFICATIONS_DESIGN.md`.
- Lints to respect: `unwrap_used`/`expect_used` deny (`Cargo.toml:84-85`), `HashMap`/`HashSet` disallowed (`clippy.toml`;
  use `BTreeMap`/`BTreeSet`), `too_many_arguments` expects carry a reason, `checked_mul`/`saturating_mul` for backoff.

## Verification

- **Before build 1, hand-review the five riskiest edits:** (1) `Event`/`batched_observed`/`recv_timeout` in `pipeline.rs`
  (explicit types, `None::<Duration>`, wrapper first as a mechanical copy; coordinate with the lane refactor's edits to the
  same file); (2) the `thumbs::run_while` Sink/Writing split (declare `Sink` first, move it back out after the call; grep
  every doc link to `first_failed`, `fold`, `Failed`); (3) migration/`io_attempts`/views/`pending_sql` (validate in sqlite3 on
  the scratch catalog first); (4) the Yandex cxx-qt changes (copy `folders_bridge.rs:68-74` exactly; re-check the role name
  against the QML); (5) the `Unreadable` marker plumbing across `pipeline`/`video`/`analysis` and the new `FilePass` matches.
- Build 1: instrumentation + everything not gated; run and read the numbers; build 2 only for what they decide.
  `just test`, `just qmltest`, `just lint` (including the doc and wasm checks above) on the final tree.
- New Rust tests as listed per item; the edited tests listed in 5.0/5.4; the `EXPLAIN QUERY PLAN` pins must stay green
  (incremental keeps `SEARCH f USING INDEX files_seq`; the full form stays off `sqlite_autoindex_files_1`; the due branch
  uses `io_attempts_due`; `files_to_import` stays the driver).
- Manual (`just run`): toggle the cache setting during a thumbnail pass (pass ends after its batch, no `CacheFull`;
  toggling back resumes; flip during the last batch then toggle back: nothing is skipped); Yandex search by photo
  (thumbnails stay put as dead results arrive, preview stays on its picture or closes cleanly if its result died);
  preview Left/Right and the wheel (previous frame stays until the next is ready, no black flash, the wheel steps in fit
  and in 1:1 mode and never scrolls the image; a 1:1 image still pans by drag and scrollbars); quit mid-import on the 190k library and relaunch (stop latency from
  `PG_PERF_LOG`; the failed list holds every recorded failure, including ones seen in the last 2 s before quitting); unplug
  a drive for several rounds (the AUDIT's own check, adapted to lanes: files do not become permanent failures; an
  import run with the drive unplugged leaves the files pending; after it is back they are retried, and `io_attempts` rows
  written by a mid-round unplug wait out `next_try_at`; the drive's lane is retired within ~25 s and the offline-file
  notice, not `UnreadableFiles`, is what shows while it is away; with `serial_drives` on, the single lane keeps
  `VolumeSet::All` and shows one `UnreadableFiles` count); a file locked for 30 s shows
  nothing in the inbox and heals; a permission-denied file gives up after 3 attempts into a visible terminal row; deleting a
  file between scans leaves no inbox row.
