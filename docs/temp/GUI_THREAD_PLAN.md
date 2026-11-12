# GUI thread plan: the window's catalog work off the Qt thread

Status (2026-11-11): a plan, not started. Seeded by [NOTIFICATIONS_FIX_PLAN.md](NOTIFICATIONS_FIX_PLAN.md) (C2), which
moved only the inbox's own reads off the GUI thread and left the rest here as a tracked deliverable. The site list was
taken from the working tree of that change by grepping `crates/desktop/src` for `app::get()`, the window's one accessor
of the `pg_app::App` (`desktop/src/app.rs`), and following each call into the service it reaches. Symbols, not line
numbers: re-read each site by symbol before acting on it.

## Why

The window does its catalog work through the `pg_app` services (`Catalogs`, `Reading`, `Views`, `Edit`,
`LibraryService`), each of which runs its closure on a pooled connection (`Catalogs::with_catalog`) on the caller's
thread. Called from a QML slot, a QML-bound getter or a Qt signal handler, that is the GUI thread. Two costs:

- **Reads** never wait for a writer in WAL mode, but the query's own time is spent on the GUI thread, and the pooled
  connections are shared with everything else the window does.
- **Writes** take the catalog's write lock and wait up to the connection's `busy_timeout` (5 s, `schema::SETTINGS`) for
  it. The scan no longer holds that lock across a pause or a listing (the fix plan's W2), but an import's flush, a scan's
  batch, the job's pairing batches and the command line still hold it for their transaction's length, and the window
  waits behind them.

The asynchronous paths already exist: the window's `Reader` (`pg_app::reader`, `app::get().reading().session()`), one
thread with its own connection, which keeps only the newest request per `(session, Kind)` and cancels a stale running
one; and the inbox's writer (`pg_app::Inbox`), whose ops are queued and applied in order.

## The sites

### Already off the GUI thread

- **The `Reader`** (asynchronous, delivered on the GUI thread): `gallery/model.rs` `open_listing` (`Session::grid`),
  `filters.rs` the facets (`Session::facets`), `folders.rs` `reload` (`Session::tree`), and since C2 `failed_files.rs` (the
  expanded ledger entry, `Session::failed_files`), `notices.rs` Copy (`Session::copy_details`) and Reveal
  (`Session::reveal_folder`, which replaces `Catalogs::present_folder` in `shell.rs` `reveal`).
- **Queued to the inbox's writer**, no catalog access on the caller: `app.rs` `post`, `resolved`, `resolved_part`;
  `notices.rs` (marks, `set_viewing`, `dismiss`, `dismiss_all`, `retry`, `poke`); `library.rs`'s pump (`poke` after the
  job's rounds).
- **Queued to the job or the writer**: `library().wake_job()` (`notices.rs` Retry), `library().save_crops`
  (`gallery/edits.rs`), `library().running()` (`library.rs`), and `library().start` (`library.rs` `start`: it starts the
  job and writer threads and queues two inbox clears; it opens no catalog itself).
- **Not on the Qt thread**: `providers.rs` (`thumbnail`, the placeholder, `full_image`, the Yandex picture) runs on the
  image providers' thread pool (`cpp/providers.cpp`, a `QQuickAsyncImageProvider`).

### Catalog reads on the GUI thread

| Site | Call | Reached from |
| --- | --- | --- |
| `gallery/state.rs` `with_details` | `reading().details` | `gallery/items.rs` `info` and `border_rejected`, QML-called getters; a one-entry memo in front |
| `gallery/state.rs` `fresh_entry` | `reading().entry` | `Grid::arrive` (`gallery/model.rs`, every listing delivered) and `refresh_turned` (`gallery/edits.rs`, a turn) |
| `animation.rs` `restart` | `catalogs().file_path` | the viewer starting an animation |
| `video_source.rs` `play` | `catalogs().file_path` | the viewer starting a video |
| `yandex.rs` `search_file` | `catalogs().file_path` | the Yandex panel's search of a file |
| `library.rs` `load_roots` | `library().roots()` | `start`, `add_path`, `remove_folder` |
| `shell.rs` `view_names` | `views().all()` | the saved views' list (after each save and delete) |
| `shell.rs` `apply_view` | `views().view` (reads them all) | the views menu |
| `controller.rs` `start` | `views().listing()`, `views().view` | the window's start (`--apply-view`) |

### Catalog writes on the GUI thread (each can wait up to 5 s)

| Site | Call | What it writes |
| --- | --- | --- |
| `gallery/edits.rs` `rotate` | `library().turn` | `queue_rotation` (the turn and its write-back) |
| `gallery/edits.rs` `set_rating` | `edit().rate` | `set_rating` (and its write-back) |
| `gallery/edits.rs` `set_border_marked` | `edit().set_border_mark` | `border_marks` |
| `shell.rs` `save_view`, `delete_view` | `views().save`, `views().delete` | `views` |
| `shell.rs` `execute` (`SaveSession`) | `views().save_listing` | `session` |
| `library.rs` `add_path` | `library().add_folder` | `add_root` (then a scan is queued) |
| `library.rs` `remove_folder` | `library().remove_folder` | the roots read, then `remove_root` (marks the folder's files missing) |
| `library.rs` `rescan` | `library().rescan` | none: a roots read, then the scans queued |

### Not the catalog, but on the GUI thread

- `settings.rs` and `shell.rs` (dev mode): `prefs().get()` is in memory; `prefs().update` writes the preferences file
  (file I/O on the GUI thread, small). `picker.rs`: `yandex().start_folder()` reads the preferences.
- `yandex.rs` `search()`: the Yandex service, whose work runs on its own threads.

## Design questions

Making the window do no catalog work at all needs its own design. The questions:

1. **A details cache as a property with a change signal.** `info` and `border_rejected` are getters QML binds to, so
   they must answer at once. They could read a cache the `Reader` fills (a `Kind` of its own for the current item, and
   its neighbours), exposed as a property with a NOTIFY signal that the bindings depend on, so a delivery re-evaluates
   them; until then the getter answers "loading". Open: what the cache holds (the current item only, or a window around
   it), and how a file change invalidates it (`forget_details` today).
2. **An asynchronous `arrive`.** `Grid::arrive` calls `fresh_entry` for the entries it must re-read (the viewed one, the
   turned ones) while it applies a listing. Either the `Reader`'s listing job reads them with the list (one delivery,
   one snapshot), or `arrive` becomes two steps (apply, then patch when the re-reads are delivered). The first is
   simpler and consistent; the second keeps the listing's query unchanged.
3. **An edit queue with an optimistic UI.** A rating, a turn or a border mark changes the grid at once and goes to a
   writer thread in order (the inbox's writer is the pattern: one work log, retried while busy, a failure reverts the
   optimistic change and posts `edit-failed`). Open: whether it is the inbox's writer, the write-back writer, or a writer
   of its own; and how a revert reaches the grid's entry (a `Change` on the bus, or the writer's delivery).
4. **A point-lookup lane on the `Reader`.** `file_path` (a video or an animation starting, a Yandex search) is one row by
   key, but behind a running listing on the one reader thread it would wait for the listing. A second lane (a thread
   and connection of its own, or a priority in the queue) keeps a video start from waiting behind a listing. The views'
   and roots' reads are small and could go there too.
5. **Views and roots.** `view_names`, `apply_view`, `load_roots` and the start's `views().listing()` are small reads; the
   start's could move before the window opens (it is read once), and the others to the point-lookup lane, with their
   writes on the edit queue.

## Relation to the scroll-freeze investigation

The scroll hitch (the grid freezing while scrolling, worst during an import or the on-demand thumbnails of a library the
thumbnail pass hasn't reached) is still open, and no document in `docs/` tracks it yet: NOTIFICATIONS_PLAN.md lists "the
scroll-hitch and thumbnail-starvation investigation" as out of its scope, and DECODE_ONCE_PLAN.md's context records the
case of tiles made on demand while scrolling (R-86 in REVIEW.md). This plan bears on it directly:

- Of the sites above, the ones that run during a scroll are `with_details` (the status line's `info`, re-evaluated as the
  current item changes) and `fresh_entry` (each listing delivered during an import). A query on the GUI thread there is
  a frame's time spent off rendering, and the details memo holds one entry.
- The edits wait up to 5 s behind a writer's transaction; a rating given while scrolling during an import is a freeze of
  up to that long.
- The image providers are already off the GUI thread, so the thumbnails' decode is not a GUI-thread cost; their contention
  for the cores is the starvation half of the investigation, not this plan's.

So the investigation should first measure (a trace stage around each site above, `pg_trace`) whether any GUI-thread
catalog call coincides with the freezes, before this plan's design is chosen: if one does, the details cache and the
asynchronous `arrive` come first; if none does, the freeze is elsewhere (layout, the scene graph, the providers'
contention) and this plan is a robustness change, not the fix.
