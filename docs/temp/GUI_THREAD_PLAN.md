# GUI thread plan: the window's catalog work off the Qt thread

Status (2026-11-13): a plan, not started. Nothing in the product code has changed. Six adversarial reviews have been
applied; the design was reworked around one writer per process after the fifth, and the sixth reviewed that rework. The
reviews' findings are not kept here, and the drafts they were applied to are not in git (the file is uncommitted). Claims
that depend on code not yet checked are listed under [Open](#open).

Seeded by [NOTIFICATIONS_FIX_PLAN.md](NOTIFICATIONS_FIX_PLAN.md) (item C2). The site lists come from grepping
`crates/desktop/src` for `app::get()` and from reading the call graph; the grep missed sites, so [Inventory](#inventory)
lists both. Sites are named by symbol. Line numbers drift: re-read each site by symbol before acting on it.

Words used with one meaning throughout:

- **the writer**: the catalog's one writing thread. The thread that writes image files is *the write-back*; the one that
  writes the preferences file is *the preferences writer*.
- **command**: an edit the GUI sends. A request to a Part 5 worker is a *request*.
- **pending**: a command sent and not yet acked. The catalog's own `files.pending` and an entry's `pending_turns` (the turn
  queued in `file_writes`) are named in full.
- **revision**: the catalog's revision of Part 1.

## Why

Measured on a copy of the live catalog (see [Measurements](#measurements)):

- **Writes wait on the write lock, up to the 5 s busy timeout** (`schema::SETTINGS`). A rating made during a 7 s hold failed
  after 5.3 s with "database is locked", and the edit was lost.
- **An import holds the write lock for its pairing.** Each 500-file flush held the lock for 2.0 to 2.5 s in release and
  12.4 s in debug. Pairing was 76 to 92% of that hold in release and 98% in debug. A GUI write during an import waits for
  it, and fails if the import holds the lock longer than the timeout.
- **A GUI turn can stall** for 0.51 to 0.84 s when its commit triggers SQLite's automatic checkpoint (the 1000-frame
  default). Once, with the library idle, a turn took 737 ms.

Reads are fast (at most 0.39 ms on a warm connection; 3.0 ms for the first read on a cold pool) and do not wait on a
writer (WAL). Opening a connection can wait: `Catalog::open` retries
the shared lock for about 5 s while another process holds it (`crates/dbfile/src/lock.rs:55-65`), and
`Catalogs::with_catalog` opens a connection on the caller when the pool is empty (`crates/app/src/catalogs.rs:38-43`).
The GUI thread must never open one.

The first two have one cause: the process has several writers, each with its own connection, and they meet on SQLite's
lock with a timeout. Today those are the job (scan, import, borders, thumbs), the write-back, the inbox and the GUI's pool.

## Goal and accepted exceptions

The GUI thread makes no catalog calls and no blocking file or lock calls. Accepted exceptions:

- the shutdown joins, which run after the window has closed;
- modal dialogs the user is waiting on (the native picker);
- the one-time startup calls that run before the first frame (the log and panic-log setup, `Config::from_env`, the
  pipewire probe, the `gpu_thread` join). Their durations are not measured ([Open](#open));
- env-gated and harness writes, which run only when the environment asks for them.

Each accepted item is listed in [Inventory](#inventory).

## Design

**Principles.**

- The catalog's connections belong to worker threads, and the window talks to them through messages. The boundary is
  asynchronous.
- A process has one writer. Every write transaction, whoever asks for it, runs on one thread, on the only connection that
  can write. Inside the process no write ever meets another on SQLite's lock, so none fails busy. Order between writes is
  a queue's order, not a race.

```text
GUI thread (Qt)                        workers
---------------                        -------
Query(req)           ───────────────►  point lane     (FIFO; own thread and read-only connection)
Query(req)           ───────────────►  listing lane   (the Reader; newest request per session and kind)
Result(req, rev)     ◄───────────────  posted to the Qt thread with the revision it was read at

Command(id, op)      ───────────────►  command thread (pg_app: coalesces, then calls the writer and waits)
Ack(id, rev, value)  ◄───────────────  posted to the Qt thread; carries what the catalog now holds

                                       write(class, stop, |tx| ...)
command thread, job, write-back, inbox ───────────────►  the writer (pg-catalog: one thread, the one writable connection)

Change(...)          ◄───────────────  the change bus
```

Components, by crate. `pg-catalog` may depend only on `dbfile`, `log`, `model`, `platform` and `query`
(`tools/crate-layers.txt`), so it knows nothing of commands, the bus, the job or the write-back.

- **`Database`** (`pg-catalog`). The one way to reach a catalog file. `Database::open` does what `Catalog::prepare` does
  today, then starts the writer and the watcher. It hands out read connections and the writer's handle. `Catalog::open`
  and `prepare` stop being public. The app, the command-line tool and the tests all go through it. It is a value the app
  owns and passes, not a `static`.
  - *One sanctioned attachment.* The thumbnail cache opens its own SQLite file and attaches the catalog to it on a
    `query_only` connection (`crates/thumbcache/src/reader.rs:24-28`, called from `crates/thumbs/src/lib.rs:180`). That
    stays, but the `Database` performs the attach, so it knows of every connection on the file.
- **Read connections** (`pg-catalog`). Every connection but the writer's is opened with `PRAGMA query_only = ON`, so SQLite
  refuses a write on it. `Catalog.conn` is `pub(crate)`, so nothing outside the crate can switch the pragma off. A read
  connection is closed by dropping it, never through `Catalog::close`: `query_only` does not block a checkpoint, and
  `close` runs `wal_checkpoint(TRUNCATE)` (`crates/catalog/src/core.rs:52-53`).
- **The writer** (`pg-catalog`). One thread with the one writable connection, held for the life of the process. It takes
  the keeper's place: the keeper is only the prepared connection held open (`crates/app/src/app.rs:62`). Its one entry is
  `write(class, stop, |tx| ...)`: the caller blocks until the transaction commits, and gets its result and revision.
  - *Every transaction begins `IMMEDIATE`*, as `write_transaction` does today (`crates/catalog/src/transaction.rs:13-16`).
  - *Three classes*, served in this order, first in first out inside each: **edits** (the command thread), **service** (the
    write-back's and the inbox's small writes), **job** (the job's passes: scan, import, borders, thumbs).
  - *Nothing is interrupted.* A new edit waits for what is in progress (one transaction or one checkpoint) and for the
    edits ahead of it, not for a queue of job writes. How long one transaction can be is in Part 1.
  - *Every call carries its caller's stop.* `stop` is a flag or a deadline. While the call is queued, and while the writer
    waits on another process's lock for it, a raised stop ends the call with `Stopped`. So a thread that is told to stop
    is never left inside `write`.
  - *No write from the writer's own thread.* A closure that calls `write` would wait on itself. The call checks the
    thread's role and returns an error.
  - *A panic in a closure* is rolled back. The writer reopens its connection, as the job does today for a panicking pass
    (`crates/library/src/runner.rs`, `round`'s caller), and the panic is resumed on the caller's thread. Each caller keeps
    its own policy: the job catches it, and the write-back thread ends and is restarted with backoff
    (`crates/writeback/src/lib.rs:340-342`).
  - *The closure may borrow.* The caller blocks until the closure has run, so the hand-off is scoped, like
    `std::thread::scope`. The flushes borrow pass-local state today (`crates/import/src/lib.rs:50-89`,
    `crates/borders/src/lib.rs:60-117`, `crates/catalog/src/pair.rs:96-229`). The lifetime erasure is one documented
    `unsafe` block in `pg-catalog`.
- **A transaction holds only SQL.** Whatever a write needs computed is computed by its caller first. Part 2a is the first
  case: the import's pairing.
- **A write decides inside its transaction.** A worker now reads on one connection and writes on another, so what it read
  may be stale by the time its write runs. A closure re-reads, inside the transaction, whatever its write depends on, as
  `store_analysis` does with `IS_CURRENT`.
- **The thread role** (`pg-platform`). A thread-local with three values: GUI, writer, worker (the default). The Qt main
  thread and the writer set theirs at start. It lives in a crate `pg-catalog` may use, so `write` can check it (R8).
- **Command thread** (`pg_app`). One thread that owns the GUI's commands. It coalesces them (R6), runs each as a
  `write(edits, ...)`, and then, on its own thread, does what follows a commit: it posts the ack to the Qt thread,
  publishes the `Change`, wakes the write-back, queues a scan, or tells the job a folder is gone (Part 4). None of that
  runs on the writer. `Command` and `Ack` carry `pg-model` types only (R10).
- **Point lane** (`pg_app`). A FIFO executor with its own thread and its own read connection, opened at startup. It serves
  details, entry, `file_path`, views and roots. It never drops a request and never waits on the writer (a WAL read). It
  keeps its connection's page cache between requests. After stop it answers each queued request with an error.
- **Listing lane** (`pg_app`). The existing `Reader`, unchanged in behaviour (`crates/app/src/reader.rs:209-238`: the newest
  request per session and kind replaces the waiting one, and cancels the running one of the same kind). Its connection
  (`reader.rs:172`) becomes a read connection. Each listing result gains the revision of its snapshot.
- **Read-ahead.** In the viewer, when the open item changes, the GUI asks the point lane for that item and for its previous
  and next, so arrow navigation finds the neighbours ready (Part 3). It replaces the two-entry details memo
  (`gallery/state.rs:56`) and is the only copy of details the GUI keeps.
- **Revision.** The catalog-wide revision of Part 1. It counts every committed transaction that changed something, from any
  process; each read returns the revision it read at.
- **The watcher** (`pg-catalog`). One thread in the `Database`, with its own read connection. With the writer, it notices
  another process's writes (R3a).
- **Qt-side values, in two layers** (`pg_client::Grid` and the desktop's model). The GUI holds the *stored* values, each
  listing with the revision it was read at, and separately the *pending* commands. What shows is the stored value with the
  pending commands applied on top. Reads and acks change only the stored layer; commands and acks change only the pending
  layer. Both layers live in `pg_client::Grid`, because the layout reads the entries there (`crates/client/src/grid.rs:109`,
  `:121`, `:198`); the viewed item's properties, the row roles and the image keys are derived from it.
- **Providers.** The image providers do not use the point lane. They read through `Images` and the catalog pool on their own
  threads, as today (`crates/app/src/images.rs`). The pool's connections become read connections.
- **Part 5's workers** (`pg_app`). A small pool. A file or volume probe that can hang (a dead network drive) runs there as
  its own task, never on the writer or the command thread and never queued behind another probe (R7). The other Part 5
  requests run there too.

## Rules

- **R1, a read never waits and never hides a pending edit.** A read result always goes to the stored layer, under R2.
  The pending layer is drawn over it, so a file with a pending command keeps showing its pending value (R4). No result is
  dropped or merged because a command is pending.
- **R2, the stored layer only moves forward, and a change that a result missed is read again.** Two mechanisms, and
  neither refuses a result because of a `Change`:
  - *Revisions.* Each listing in the model carries the revision it was read at (`Grid.entries`, and the listing held under
    the viewer, `crates/client/src/grid.rs:109`, `:121`). An entry replaced on its own since (by an ack or a point read)
    carries its own revision in a sparse map beside the listing; the map is emptied when a new listing is applied. A
    result, an ack or a listing row applies only if its revision is at least the held one for that entry. A refused
    result is always older than what is held.
  - *Changes noted while a request is in flight.* While any read is in flight, the GUI notes each `Change` it receives:
    the file and its revision, or "everything" and its revision. When a result arrives, every entry in it whose noted
    revision is above the result's revision is asked for again, and so is a first read that was in flight when its file
    changed. The notes are dropped when no read is in flight. This generalizes what the grid does today for turns
    (`note_turns_written`, `take_turned` and `refresh_turned`, `crates/client/src/grid.rs:345-354`,
    `desktop/src/gallery/model.rs:158-160`), and it covers the case a revision check alone loses: a file changes while a
    listing that contains it is loading, and the older snapshot then lands.
  - A `Change` for a file whose held revision is already at least the change's (the ack arrived first) asks for nothing.
  - A result's revision is the one its own read transaction saw (Part 1), so it bounds the data the result holds.
- **R3, lost events.** The bus drops the oldest event for a lagging subscriber and counts the drops
  (`crates/jobs/src/events.rs:82-85`). Every pump reads that count (Part 3). When it is above zero, the GUI refreshes
  everything (below). The results apply under R2.
  The inbox's `Changed` events carry the whole listing, and the pump keeps only the newest (`desktop/src/notices.rs:261-265`),
  so a lost one costs nothing. A lost `Toast` is dropped silently (the inbox bus holds 256 events,
  `crates/app/src/inbox.rs:303`); the pump logs the dropped count. The window shows only the last toast of a pass anyway
  (`desktop/src/notices.rs:294`).
  - *"Refresh everything"* is one list, used by R3, R3a and the pump's first pass: the grid's listing and the folder tree
    (through the listing lane, as `Scanned` does today, `desktop/qml/Main.qml:127-128`, `:255-265`), the facets
    (`Main.qml:142-145`), the open item and its neighbours, the views, the roots, the status lines, and the inbox's poke
    (`desktop/src/library.rs:134-160`).
- **R3a, writes from another process.** The command-line tool writes the same catalog through its own `Database` and its
  own writer: scan, import, remove, the pending marks, borders, thumbs and the write-back (`crates/cli/src/lib.rs:218`,
  `:228`, `:248`, `:279`, `:324`).
  - *`data_version` alone is not the test.* It moves whenever any other connection commits
    (`crates/app/src/inbox.rs:716`), and that includes this process's writer.
  - *The test is a revision this process did not write.* The `Database` keeps the last revision it has seen. Two places
    compare the stored revision with it:
    - the writer, inside every transaction, before it bumps. If the stored revision is not the last seen, another process
      committed in between, and the writer reports it after its own commit. This catches a foreign write that an own
      commit follows before the watcher looks;
    - the watcher, which reads `data_version` once a second (it costs microseconds) and, when it has moved, reads the
      stored revision. If that is above the last seen, it reports it. This catches a foreign write while the writer is
      idle.

    Both raise the last seen revision. A report made twice costs one spurious refresh, which is harmless.
  - *The route.* The `Database` calls a function the app gives it. The app publishes `Change::Foreign { revision }` on the
    change bus, and the library pump answers it by refreshing everything (R3). The app creates the bus in `App::open`;
    today `Library::start` creates it (`crates/app/src/library.rs:99`).
  - *The inbox is not changed by this.* Its own `data_version` comparison (`inbox.rs:803-804`) also tells it of this
    process's ledger writes (`inbox.rs:8`), and it keeps it.
  - Another process's writes therefore reach the window within about a second.
- **R4, pending is shown, and nothing looks done before its write commits.**
  - *A pending value is dimmed*, and assistive technology is told it is pending. The dimming starts only after the
    command has been pending for 200 ms: an ack normally lands on the next loop turn, and dimming at once would flicker
    on every edit. The 200 ms is chosen, not measured.
  - *The ack* carries what the catalog now holds, read inside the command's own transaction, so it is one snapshot at the
    ack's revision: the file's entry for a file command, the views' names for a view command, the roots for a folder
    command (for a removal, also the removed root's path). It goes to the stored layer under R2, and the command leaves
    the pending layer. A command that changed nothing is acked with the revision it read.
  - *A pending rating or border mark* shows its new value at once, dimmed after 200 ms.
  - *A pending turn does not turn the picture.* The image key is `<id>@<mtime>.<rotation>.<pending_turns>`
    (`desktop/src/gallery/items.rs:41-46`), and on a thumbnail miss, the placeholder and the full image the providers turn
    by what the catalog holds (`crates/app/src/images.rs:227`, `:272`, `:290`). A key that ran ahead of the catalog would
    make Qt cache unturned pixels under the turned key. So the key and the layout's aspect (`Entry::aspect`,
    `crates/model/src/grid.rs:75`) come from the stored layer only. A pending turn shows as the pending mark alone.
  - *A turn has two later moments.* At the ack, the stored entry holds the queued turn (`rotation` is `files.rotation`
    and `pending_turns` is the queued `file_writes` value, `crates/catalog/src/browse/read/grid.rs:26-27`, `:68-69`), the
    key changes, and every view turns, as today after `queue_rotation` commits (`desktop/src/gallery/edits.rs:27-47`).
    At `Turned`, the write-back has rewritten the file, and the entry is read again.
  - *A command that fails* (a constraint error) leaves the pending layer, and the stored value shows again. No read is
    needed: the stored layer never held the failed value.
  - *A rating does not re-list.* A file rated out of the current filter stays until the next reload, and the facets stay
    as they were (`crates/client/src/grid.rs:393-395`, `Main.qml:260-262`). That is today's behaviour, kept.
- **R5, the lock is busy only when another process holds it.** Inside the process the writer is alone, so a busy write
  means the command-line tool holds the lock.
  - *The wait.* The writer's connection has a busy handler in place of the 5 s timeout (rusqlite's `busy_handler` takes a
    closure, `src/busy.rs:55-57`). It sleeps and tries again, and gives up when the current call's stop is raised or the
    writer is told to stop. It is installed after `SETTINGS`, because `PRAGMA busy_timeout` replaces it
    (`crates/catalog/src/schema.rs:377-378`; so does `Catalog::set_busy_timeout`, `core.rs:190`). No write is lost to a
    timeout.
  - *The busy state.* After 10 s in that wait the window shows a busy state: a QML property set on the Qt thread, not a
    database notice, since a notice is a write the same lock would hold. A transient toast says so, and the state clears
    on commit.
  - *The cap is the GUI's.* The pending layer holds at most 10,000 commands. A command beyond that is refused on the Qt
    thread, before it is sent. The value is chosen, not measured.
  - A command that fails for another reason (a constraint error) fails at once. Shutdown is in [Shutdown](#shutdown).
- **R6, coalescing per file and operation.** The command thread coalesces the commands still waiting in its own queue, per
  (file, operation) pair, not per file alone. Commands that name no file (a saved view, the session, a folder) coalesce
  per operation and key: the session keeps the last value, and a view keeps the last command for its name.
  - turns add up, mod 4, as `queue_rotation` does (`crates/catalog/src/queue.rs:103-125`); a sum of zero is still sent, so
    its commands are acked;
  - a rating or a border mark keeps the last value.
  Every coalesced command gets the one ack of the write that carried it. If that write fails, they all fail.
- **R7, a multi-step operation is one command, its probes run first, and its write is one transaction.**
  - *Adding a folder.* Normalize and the NTFS checks (`check_folder`, `crates/scan/src/lib.rs:281-283`, which calls
    `refusal::check_root`) run first, as a probe task (Part 5's workers). When they pass, the task sends the command, whose
    whole work is one short write transaction (`add_root`, `crates/catalog/src/roots.rs:39-53`, which also deletes the
    roots nested in the new one). After the commit the command thread queues the scan (`Runner::scan`).
  - *Removing a folder.* One command, one transaction (`remove_root`, `crates/catalog/src/roots.rs:58-72`), which deletes the
    root row, sets `missing = 1` over the folder's files, and cascades the root's notices and folder-ledger rows by foreign
    key. Today `remove_folder` reads the roots in a separate statement first (`crates/app/src/library.rs:315-322`) and
    passes the root's path to `Runner::folder_removed`, which stops a scan under it. The command drops the outer read:
    `remove_root` reads the path inside its transaction and returns it, and the command thread calls `folder_removed` with
    it after the commit. The measured cost is 2.3 to 3.6 s for the 189,519-file root and 0.9 to 1.0 s for the 41,293-file
    root (debug, one machine). Every write queued during it waits for it; none fails. A rating made then stays pending
    for that long. Whether one writer makes it safe to split the removal into shorter transactions is not designed
    ([Open](#open)).
  - *Rescanning* is not a catalog write. It reads the roots at a revision (a point-lane request) and queues the scans in the
    library job.
- **R8, blocking calls are checked.** The blocking entry points move into a `providers` module of `pg_app`, which the GUI
  modules do not call. The desktop's providers call them (`crates/desktop/src/providers.rs:44-119`). The facade checks the
  calling thread's role on every blocking call, in every build, and `write` checks it in `pg-catalog`: it refuses the GUI
  thread and the writer's own thread. The image providers' Qt pool threads (`Job`,
  `crates/desktop/cpp/providers.cpp:98-120`) run `Job::run()` and call into Rust, so they are workers and may block. A
  blocking call from the GUI thread logs an error and returns the failure the caller already handles. It does not panic: a
  panic across the cxx-qt bridge aborts the process.
- **R9, startup.** `app::open` runs on a startup worker, which then reads the start state. The window shows a loading
  placeholder until both are done.
  - *The start state is read on the worker, not queried by the GUI.* `ShellRust::default` returns the listing, the panels
    and the size synchronously (`crates/desktop/src/shell.rs:39-56`), from `Core::new`, which needs the session's listing,
    the applied view and the preferences before it builds the controller (`crates/desktop/src/controller.rs:38-66`).
    `Filters`, `FacetChoices` and `FolderModel` reach the same core (`filters.rs:32`, `facet_choices.rs:54`,
    `folders.rs:236`). There is no "loading" controller, and the plan adds none. The worker reads the session's listing,
    the applied view, the views' names and the preferences, and leaves them in a slot beside the app. `Core::new` and
    `view_names` take them from the slot. A failed read gives the default state and a notice, as now.
  - *The app stays a process-wide value.* The bridge objects are QML elements that QML constructs through `Default`
    (`#[qml_element]` in every `*_bridge.rs`), so they cannot be handed the app when they are made. `app::get()` stays, as
    `APP.get().expect("app::open not called")` (`crates/desktop/src/app.rs:44-51`).
  - *So no bridge object that reaches the app may exist before Ready.* Today `app::open` runs on the main thread before
    the GUI application exists (`crates/desktop/src/lib.rs:164`). During QML load the app is reached by `Shell` (in
    `Default`), `Settings` (on first property read, `settings.rs:22`, `:26`), `Notices` (`start`, `notices.rs:254`),
    `FolderModel` (`reload`, `folders.rs:180`), and `Filters` and the six `FacetChoices` (through the core). `Library`
    reaches it later, from `startHooks`.
  - *One engine load, with a `Loader`.* The root window creates one bridge object, `Startup`, which reaches nothing. A
    `Loader` creates the main content when `Startup` reports Ready. The app and the start state are set before Ready is
    posted, so the `expect` cannot fail by construction. The worker may finish before or after QML constructs `Startup`,
    and live reload constructs it again (`cpp/live_reload.cpp:100`), so `Startup` reads the state from the slot as well
    as being told.
  - *All of `Main.qml`'s content moves into the loaded component.* What the root must keep or forward: the title (bound
    to `shell.folder`, `Main.qml:21`); `current` (an alias into the model, `:47`, which cannot reach into a `Loader`);
    `ready` and `shellCore`, which `cpp/screenshot.cpp:82`, `:177` read (`:74`, `:116`); `onClosing`, `onActiveChanged`
    and `onDevicePixelRatioChanged` (`:138`, `:369`, `:734`); and `Component.onCompleted` (`:736-748`). The `firstFrame`
    trace stage (`:117-124`) moves to the loaded content, or it would time the placeholder.
  - *A failed open.* Today it is fatal before Qt exists: a native error box, then `process::exit(1)`
    (`crates/desktop/src/lib.rs:164-166`, `alert.rs`). There is no app and so no inbox for a notice. The worker reports
    the failure in place of Ready, and the GUI thread shows the same box and exits.
  - *A close before Ready* waits for the worker, then runs the ordinary shutdown. `app::stop` and `app::shutdown` already
    tolerate a missing app (`crates/desktop/src/app.rs:55`, `:64`).
  - Once loaded, the first content frame shows the start fields as today. So R9 is a cost the goal forces, not a fix for a
    visible flash.
- **R10, visibility.** The public surface of `pg_app` is its message types and its facade. The catalog data types the front
  end needs move to `pg-model`, which every crate may depend on: `Facets`, `pg_catalog::inbox::Entry` (aliased `InboxEntry` in
  `crates/app/src/reexport.rs:10`), `FailedFile`, `FailedFolder`, `FolderFailure`, and the listing and inbox data. Then
  `Catalogs`, `Reading`, `Views`, `Edit`, `LibraryService` and `Session` become `pub(crate)`, and `reexport.rs` is removed
  with its allowlist entry in `tools/boundaries.py` (line 35). The other items that file passes to the front end move to
  `pg-model` too: `Change` and `Cause` (from `pg_jobs`), `WriteError` (`pg_writeback`), `ThumbCap` (`pg_thumbcache`) and
  `InboxCounts` (`pg_catalog`). `pg-model` is the leaf of `tools/crate-layers.txt` and already holds `FileId`; `jobs`,
  `library`, `writeback`, `app` and `desktop` all list it, so `Change` moves there with no layering violation. Part 6 adds
  a check that no `pub` item of `pg_app` names a `pg-catalog` type. `tools/boundaries.py` checks crate-layer edges, dev
  edges, re-exports and platform `cfg`, not item visibility, so the check is needed.

## Build order

Parts are in dependency order, and the app works after each one.

- Part 1 comes first because everything else stands on it: the revision is bumped in one place only once there is one
  writer. It includes the command thread, because Part 1 makes the pool read-only and the GUI's writes need somewhere to
  go.
- Parts 1 to 3 come before the GUI stops waiting (Part 4): a write whose result the GUI cannot read back consistently is
  worse than a write that waits.

### Part 1, the `Database`: one writer, read-only connections, the revision

**The `Database`, the writer and the command thread** are as in [Design](#design). Every site that opens a connection or
begins a write transaction today moves onto them:

| Today | Site | Becomes |
| --- | --- | --- |
| the job's connection, kept between rounds | `round`, `crates/library/src/runner.rs` | a read connection; each pass's transactions are `write(job, ...)` |
| the scan | `lease.tx()`, `crates/scan/src/lib.rs:623`; `begin_scan`, `finish_scan`, `mark_missing` and `folder_refused`, `crates/catalog/src/roots.rs:79-163`; the pairing tiers, `crates/catalog/src/pair.rs:266-283` | job writes; the lease is below |
| the import | `catalog.tx()`, `crates/import/src/lib.rs:51` | one job write per flush (Part 2a) |
| the borders pass | `catalog.tx()`, `crates/borders/src/lib.rs:61` | job writes |
| the thumbnail pass | its own `Catalog::open`, `crates/thumbs/src/lib.rs:202`; the cache's attach, `crates/thumbcache/src/reader.rs:24-28` | a read connection, and the `Database`'s attach; its only catalog writes are the failure ledger's (`record_failures`, `clear_failures`), which are job writes |
| the write-back | its own connection, `crates/writeback/src/lib.rs:326`; `fault.rs:73-74` | a read connection; `write_done` and the queue's writes are service writes |
| the inbox | its own connection, `crates/app/src/inbox.rs:742` | a read connection; each step stays its own transaction, as today (`inbox.rs:813-849`), and is one service write |
| the listing lane | `crates/app/src/reader.rs:172` | a read connection |
| the GUI's pool writes | `crates/app/src/edit.rs:29`, `:36`; `views.rs:42`, `:52`, `:57`; `library.rs:300`, `:315`, `:338` | commands (below) |
| the pool | `Catalogs::with_catalog`, `crates/app/src/catalogs.rs:38-43` | read connections only |
| the keeper | `pg_library::open_catalog`, `crates/app/src/app.rs:62` | `Database::open`; the writer's connection takes its place |
| the command-line tool | `pg_library::open_catalog`, `crates/cli/src/lib.rs:136` | `Database::open` |

Crops are not in the table: `pg_crop::save` only reads the catalog (`crates/crop/src/lib.rs:68-78`).

**The GUI's writes, until Part 4.** The GUI's calls keep their signatures. Each sends its command to the command thread and
waits for the ack on the GUI thread, for at most 5 s, which is what today's busy timeout allows. So the app behaves as
today after Part 1. Part 4 removes the wait, and Part 6's check then forbids it.

**The preparation's writes go through the writer.** `Catalog::prepare` writes today, each in its own immediate
transaction: `mark_unheld_missing` and the import, borders, thumbs and taken-zone marks (`crates/catalog/src/core.rs:115-119`).
They run even when another process has the file open; only a rebuild or a migration needs the exclusive hold
(`core.rs:91-95`). In `Database::open` they are the writer's first transactions, so they bump the revision and a running
window is told (R3a). `set_import_version` rewrites every file row twice (`crates/catalog/src/lifecycle.rs:76-84`), so a
second process starting can hold the lock for seconds.

**The scan's lease.** `ScanLease` holds `&mut Catalog` and derefs to it (`crates/catalog/src/core.rs:247-291`); the walk reads
and writes through it. It guards against other processes and crashed walks, with a 60 s expiry renewed every 20 s
(`crates/catalog/src/types.rs:89`, `crates/scan/src/lib.rs:142`), so it is still needed.

- It becomes a value over a read connection and the writer's handle. Its reads use the connection and its writes are job
  writes.
- *Its release is a call, not a `Drop`.* Today `Drop` writes (`core.rs:293-297`). A blocking `write` in a destructor would
  run during an unwind and after the writer has stopped. A lease that is dropped without the call expires by itself.
- *A write that waited may have lost the lease.* A job write can now wait behind edits, service writes and another
  process. `renew` already detects a lost lease (`scan/src/lib.rs:600-606`), but the per-batch heartbeat ignores its row
  count (`crates/catalog/src/writers.rs:83`), so a batch can commit after expiry. The heartbeat checks the count inside the
  transaction and rolls the batch back when the lease is gone.

**Writes cannot be made anywhere else.** Two things enforce it, and neither is a grep:

- every other connection is `query_only`, so SQLite refuses the statement;
- the write functions take the transaction, not the connection, and only the writer makes one. A write outside a
  transaction does not compile. Today these run on a bare connection and are converted:
  - `set_border_mark` (`crates/catalog/src/borders.rs:22-30`);
  - `save_view` and `delete_view` (`crates/catalog/src/browse/views.rs:26-39`), and `save_session_listing`
    (`crates/catalog/src/browse/views.rs:50`);
  - the failure ledger (`crates/catalog/src/ledger.rs:250-258`, through `ledger.rs:382-406`). `mark_ledger_read_on` and
    `dismiss_ledger_on` are also called inside the inbox's transaction (`crates/catalog/src/inbox.rs:226`, `:262`), so
    they take the caller's transaction and never begin one;
  - the queue's writes: `write_failed`, `drop_gone_write` and `retry_write_copies` (`crates/catalog/src/queue.rs:283-309`),
    and `record_write_temp` (`queue.rs:214`), which sets the "being written" state that `IS_CURRENT` reads;
  - `finish_scan` (`crates/catalog/src/roots.rs:105`);
  - `set_mark` (`crates/catalog/src/lifecycle.rs:105`), which opens its own immediate transaction. It backs the
    preparation's marks above, and rewrites `files.pending`, `seq` and `analysis.taken`;
  - the scan lease's direct renewal and release (`crates/catalog/src/core.rs:264-271`, `:302-308`).

  What stays outside: the rebuild's `copy_tables` (`crates/catalog/src/rebuild.rs:186`), which fills a replacement file
  under the exclusive hold before the writer exists, and the `PRAGMA` statements.

**Tests and fixtures.** The fixtures are gated by `feature = "fixture"`, not `cfg(test)` (`crates/catalog/src/lib.rs:21`,
`transaction.rs:42`), and the app's tests call them on pooled connections (`crates/app/src/images.rs:344-349`, `:401`,
`:434`, `:469`; `app.rs:238`). Under the feature they write through `write`, like everything else. The hold timing the
tests read is a thread-local today (`WATCHED`, `transaction.rs:47-50`; the 200,000-file scan test asserts on it,
`crates/scan/src/lib.rs:1318-1324`); it moves to the `Database`, since the holds are now on the writer's thread.

**How long one transaction can be.** A new edit waits for the transaction in progress. The known ones:

| Transaction | Size | Hold |
| --- | --- | --- |
| import flush | 500 files (`crates/import/src/lib.rs:18`) | 0.19 to 0.56 s once Part 2a is in; 2.0 to 2.5 s before it (release) |
| scan batch | 10,000 entries, and a larger folder is one batch (`crates/scan/src/lib.rs:136-138`) | not measured; the test's bound is 3 s in debug (`:1306`) |
| borders | 500 (`crates/borders/src/lib.rs:22`) | not measured |
| pairing tiers, `mark_missing` | 1,000 (`crates/catalog/src/types.rs:85`) | not measured; tier 2's first batch runs a `SELECT DISTINCT volume` inside it (`pair.rs:128-131`) |
| thumbs ledger | 200, or every 2 s (`crates/thumbs/src/lib.rs:27`, `:30`) | small |
| `remove_root` | one `UPDATE` over the folder | 2.3 to 3.6 s (debug, the larger root) |
| the preparation's marks, in another process | every file row, twice | seconds; not measured |

The unmeasured holds are in [Open](#open).

**The revision.** A migration adds one `meta` row holding it (one `Step::Sql`; a migration already runs in one write
transaction, `crates/catalog/src/schema.rs:401-435`). The existing `meta.seq` (`crates/catalog/src/sql.rs:17-19`) is not it:
that is the thumbnail clock, bumped by triggers and `set_mark` and read by the thumbnail cache.

- *The bump is the writer's, at commit.* A transaction holds many writes (the import stores 500 analyses and their pairs
  in one), so the writer bumps once, just before `COMMIT`, in the same transaction as the data.
- *Only when something changed.* The writer compares the connection's `total_changes` before and after the closure
  (rusqlite exposes it, `src/lib.rs:1064`). It counts rows changed by triggers and foreign-key actions too, and it never
  under-counts a data change; it over-counts an `UPDATE` to identical values, so a lease heartbeat bumps. A transaction
  that changed no row commits without a bump, so it writes nothing, and `data_version` does not move.
- *A migration bumps explicitly.* `total_changes` does not count DDL, so each migration's transaction bumps the revision
  itself.
- *It is returned.* Every `write` returns the revision, and the writer raises the `Database`'s last seen revision after
  the commit (R3a). A commit that fails after the bump rolls the bump back with the data.
- *A rebuild.* `copy_tables` already carries a `meta` key that both files have (`crates/catalog/src/rebuild.rs:205-209`),
  so the revision is carried once the new file's seed has the key. A rebuild that carries nothing starts from the seed
  (`rebuild.rs:73-83`); that is harmless, because a rebuild needs the exclusive hold, so no other process and no window
  is reading.

A read returns its data and the revision it was read at inside one deferred read transaction,
so both come from one snapshot. A read-only transaction is not blocked by a writer in WAL mode.

### Part 2, the import's pairing and the checkpoints

**2a, the pairing is computed outside the write transaction; the index changes only after commit.**

Today the import (`crates/import/src/lib.rs`) runs each 500-file batch as: open the transaction; `store_analysis`;
`pair_and_insert` (query, then insert) inside it; `store_pdq_pair`; commit. With one writer, that transaction would run on
the writer's thread, and its pairing (76 to 92% of the hold) would keep every other write waiting. The design replaces it:

- **Scratch split.** `PdqIndex::best_matches` and `query` (`crates/dedup/src/pdq_index.rs:283-345`, both `&mut self` today)
  take `&self` and a caller-owned `QueryScratch { seen: Vec<u32>, query_no: u32, found: Vec<(FileId, u32)> }`. Today `seen`
  and `query_no` are fields of the index (`pdq_index.rs:53-54`, mutated at `:308-322`); `found` is already a caller's buffer
  (`Pairer.found`, `crates/dedup/src/pairer.rs:13`) and moves into the scratch with `Pairer.best` (`:15`).
  `QueryScratch::ensure(len)` sizes `seen` to the index. `Pairer` keeps the index only.
  `Pairer::pairs_in_index(&self, scratch, id, orientations)` returns each neighbour's best (distance, orientation), self
  excluded. `Pairer::insert(&mut self, id, orientations)` is the post-commit insert.
- **Batch matching, on the job's thread, before the write.** The candidates are the batch's files with a usable dihedral
  (today's `quality_ok` test, a local in `crates/import/src/lib.rs:61`), in the batch's own order. That is the order the
  items reached the sink (`crates/jobs/src/pipeline.rs:259`), which is the order today's loop pairs them in. For candidate k:
  its pairs against the index, plus its pairs against each earlier candidate j < k of the same batch. For j, a distance `d`
  between an orientation `v` of k and an orientation of j within `MATCH_DISTANCE` gives j's best `(d, v)`: the rule
  `best_matches` applies to stored entries.
- **The write.** One job write. Store the analyses (`store_analysis`, `crates/catalog/src/writers.rs:300-303`,
  `IS_CURRENT`). Store a pair only when both of its files were stored. A rejected file's pairs are then dropped, and no
  other pair changes, because a pair is a relation between two files. `store_analysis` deletes every pair naming its file
  (`writers.rs:306`, and `store_failure` at `:347`), so the batch's pairs are written after both files' analyses, in the same
  transaction. The write returns which files were stored.
- **After commit.** Insert the stored candidates' orientations (`Pairer::insert`). The room is checked before the
  write: `u32::try_from` of the index's length plus the orientations must succeed, the u32 link limit. (`reserve()`
  only grows capacity, `pdq_index.rs:199`.) So this insert cannot fail with `IndexFull` (`pdq_index.rs:213`, `:274`).
- **Equivalence.** Sequential query-then-insert pairs each new file with the earlier stored files. Batch matching gives the
  same pairs for two reasons, and neither is the order test:
  - the index is exact, not approximate (multi-index hashing with `MAX_RADIUS = 31 = MATCH_DISTANCE`,
    `pdq_index.rs:1`, `crates/model/src/pdq.rs:7`), so a brute-force comparison inside the batch finds what a query of the
    index would. `a_built_index_finds_exactly_the_brute_force_set` (`pdq_index.rs:452`) tests that;
  - the later file is the query in both, so the stored orientation is the same. The variant is the query file's
    orientation, ties keep the lowest (`d < b.1`, `pdq_index.rs:295`), and `store_pdq_pair` orders the ids but not the
    variant (`crates/catalog/src/writers.rs:362`).

  A rejected file's pairs are not stored in either: today a rejected file skips pairing (`import/src/lib.rs:57-60`).
  `pairs_do_not_depend_on_import_order` (`pdq_index.rs:386-414`) does not cover this. It compares only the set of id pairs
  across sequential orders, on `PdqIndex`, without distances or variants.
- **A neighbour that changed.** "Both files stored" covers the batch's own files. A pair with a file already in the index
  is stored even if that file changed between the matching and the commit. Whether today's import has the same window,
  and whether the write should re-check the neighbour, is not traced ([Open](#open)).
- **Failure.** A commit that fails leaves the index unchanged, because nothing was inserted. The run ends at that batch, as
  today; today the index is simply dropped with the run.
- **Busy.** The import has no busy policy of its own. Its write waits in the writer's queue, and the writer waits on
  another process until the job is told to stop (R5). Today a busy batch fails after the 5 s timeout and ends the run
  (`pipeline.rs:262`).
- **Cost.** Batch matching is in memory: about 125,000 pairs of 64 orientation comparisons per 500 files. Not measured. The
  writer's hold is the SQL alone: flush minus pairing was 0.19 to 0.56 s per batch (second pass, release).
- **Tests.** A failed commit leaves the index unchanged. A pair naming a rejected file is not stored. A new equivalence
  test: the same files, imported one per batch and all in one batch, store the same pairs with the same distance and
  variant, including two files of one batch that match each other. The order test still holds.

**2b, the checkpoint is the writer's lowest-priority work.**

**Precondition.** Only the first 1000-frame checkpoint of each run stalled; about 20 later ones in the same run did not
([Measurements](#measurements)). Count and time every checkpoint first ([Open](#open)). If checkpoints do not stall as a
rule, SQLite's automatic checkpoint stays as it is and this part is not built.

If they do stall, an automatic checkpoint runs on the writer's thread after a commit and delays whatever is queued behind
it, an edit included. So the writer decides when:

- The writer's connection registers a WAL hook (`Connection::wal_hook`, verified in the locked rusqlite revision `9d76035`
  behind the `hooks` feature, `Cargo.toml:36`; it takes `&mut Connection`). Registering a hook replaces SQLite's default
  hook, which is the automatic checkpoint (`sqlite3_wal_autocheckpoint` is itself a `sqlite3_wal_hook` call in the bundled
  `sqlite3.c`), so no pragma is needed. The hook receives the frame count after each commit and only records it. It does
  no SQL and always returns `Ok(())`, since an error becomes an SQLite error code.
- Read connections never commit a write, so they never run an automatic checkpoint, and they are closed by drop
  ([Design](#design)), so they never run the close's `TRUNCATE` either. The rebuild's connection has no hook; it ends with
  `wal_checkpoint(TRUNCATE)` and a close (`crates/catalog/src/rebuild.rs:119-120`).
- Whenever the recorded count is 1000 frames or more (SQLite's default) and no edit is queued, the writer runs
  `PRAGMA wal_checkpoint(PASSIVE)` before anything else, and also when its queue is empty. So an idle writer and a session
  of edits alone still checkpoint. An edit that arrives during the checkpoint waits for it. A passive checkpoint reports
  "busy" as a column of its result row, not as an error, so the writer reads the row.
- *The bound is the automatic checkpoint's, no better.* The writer is the only one in this process that appends, and it is
  not appending while it checkpoints. But a passive checkpoint cannot pass a frame that a reader's snapshot still needs,
  and the WAL restarts only when no reader is in it. A reader that holds one snapshot for long pins it: the thumbnail
  cache's pending query holds one read transaction over both files (`crates/thumbcache/src/reader.rs:29-62`), and a pass
  that keeps a cursor open across its own writes would do the same. Another process appends too. All of that is true of
  the automatic checkpoint today.
- A file size is not used as the trigger, because the WAL file keeps its size after a checkpoint.
- The close still runs `TRUNCATE` (`crates/catalog/src/core.rs:52-53`), on the writer's connection, last
  ([Shutdown](#shutdown)).
- Measured (first observations; see [Measurements](#measurements)): with auto-checkpoint on, a commit at 1000 frames stalled
  0.51 to 0.84 s, in four runs. With it off, the slowest of 3,000 turns took 3 to 5 ms, and the WAL grew to 90 MB.
- **Tests.** Both read the frame count the hook reports, not the file size, which never shrinks. Both run with no other
  reader open.
  - Commits of a few frames each cross 1,000 frames, then the writer goes idle. The next commit reports a frame count
    below 1,000 (the WAL restarted).
  - An edit is queued when the count crosses 1,000. The edit's ack comes before the checkpoint runs.

### Part 3, the read side

- **Point lane**: as in [Design](#design). Each request runs in one deferred read transaction (Part 1).
- **Listing lane**: the existing `Reader`, unchanged in behaviour; its results gain the revision of their snapshot.
- **The bus and the watcher's route.** `App::open` creates the change bus and gives the `Database` the function that
  publishes `Change::Foreign` (R3a).
- **Change payloads.** The bus carries no payload today. `Change` is a fieldless enum (`crates/jobs/src/events.rs:127-141`:
  `Progress`, `Scanned`, `Imported`, `Round`, `Writes`, `Turned`), handed out as `Bus<Change>` by `Library::changes`
  (`crates/app/src/library.rs:160`). The bus holds 256 events per subscriber (`library.rs:99`). The design keeps `Change`
  `Copy`, which it derives today, so the bus can clone it cheaply.
  - `Change::Turned { file: FileId, revision: u64 }`. Today `Turned` names no file (`crates/writeback/src/lib.rs:207-209`),
    and the write-back publishes it once per file. Verified: the write-back's `job` is in scope at all three call sites of
    `pixels_changed` (`crates/writeback/src/lib.rs:611`, `:679`, `:695`), so `pixels_changed` takes `job.file_id`. The
    revision is the one the write-back's `write_done` (`crates/catalog/src/queue.rs:224-229`) returned from the writer; it
    precedes each call. At `:611` it runs inside `copy_and_done`, which returns `(i64, i64)` today, so the revision is
    threaded out of it. The desktop match (`desktop/src/library.rs:57`) changes to `Turned { .. }`.
  - `Change::Data { file: FileId, revision: u64 }`, published by the command thread after a file command commits. A
    command that names no file publishes nothing: its ack carries the new list (R4), and there is one window.
  - `Change::Foreign { revision: u64 }`, from the watcher's route (R3a): refresh everything.
  - `Scanned`, `Imported`, `Round` and `Writes` each gain the revision of the commit they follow, and keep their handling:
    `Scanned` reloads the tree and the grid, and `Imported` re-lists, once per pass (`Main.qml:127-128`, `:255-265`;
    `crates/library/src/runner.rs`, `Imported`). The job's per-batch commits publish nothing more: a `Data` event per
    500-file batch would re-read the screen every few seconds for what `Imported` already does.
  - `Progress` is unchanged.
- **Pumps.** Every pump waits with `recv()` for its first event, then calls `take()`. It handles the event it received, then
  everything `take()` returns, in order, then the lag count. `take()` drains the queue, so its events are not discarded.
  `take()` alone does not wait, so it cannot be the wait. A pump whose events carry nothing may discard them, as the Yandex
  pump does (`desktop/src/yandex.rs:127-128`: its change is `()`, since a poll reads the whole state). A non-zero lag count
  is the R3 refresh.
  - *Today no pump reads the lag count.* The library pump takes one event per `recv()` and folds it into flags
    (`desktop/src/library.rs:114-122`, `pump.rs:51-64`); the notices pump does the same on its own thread
    (`notices.rs:260-277`); the Yandex pump discards what `take()` returns. `take()` is the only reader of the count, and
    reading it resets it (`events.rs:103-106`). The log panel's pump uses an unbounded channel and has no lag
    (`log_model.rs:124`). `pump::start`'s `recv: FnMut() -> Option<C>` changes to return a batch and the lag count, which
    changes the Yandex pump's call too (`yandex.rs:126-130`).
  - *Folding per-file events.* The library pump's flags (`Changes`, under `Pending`, `desktop/src/library.rs:43-62`,
    `pump.rs:14-25`) gain a set of (file, revision). Above 256 files in one pass the set collapses to "refresh
    everything". A bulk edit of more files than the bus holds lags the subscriber and ends the same way.
  - *The set's route to the grid.* Today the pump reaches the grid through a QML signal and the controller
    (`Main.qml:130`, `Event::FilesChanged`, `Command::RefreshTurned`, `gallery::post`; `crates/client/src/controller.rs:327`,
    `desktop/src/shell.rs:495`), a chain that carries no payload and is recorded for replay. The set does not go through
    it: the `Library` object hands it to the gallery model in Rust (`gallery::post`), and the signal stays as it is.
  - *The pump's first pass refreshes everything.* The library pump exists only after `library().start()`, which runs from
    `startHooks` after the first list (`desktop/src/library.rs:105-122`, `Main.qml:752-757`). A `Change` published before
    it subscribed is not delivered, so its first pass counts as lagged. Acks do not depend on the pump: the command thread
    posts them to the Qt thread itself.
- **Read-ahead.** In the viewer, when the open item changes, the GUI queues point-lane reads for it and for its previous
  and next. Their results apply under R2. A details or entry read measured 0.08 to 0.12 ms (p50) and at most 0.39 ms. The
  grid's arrows move by row and read details for the selection (`refresh_derived`, `gallery/model.rs:264-272`); those are
  single point-lane reads.
- **Startup.** `app::open` and the start state's reads move to the startup worker (R9), so the GUI thread opens no catalog.

### Part 4, the GUI stops waiting

Ratings, turns, border marks, saved views, the session and the folders are already commands (Part 1). Here the GUI stops
waiting for them: it sends, shows the pending layer, and takes the ack when it is posted (R4 to R7). An add's probes run
before the command is sent, as a probe task (R7). A rescan is not a command (R7).

After a commit the command thread does what the GUI call does today, on its own thread:

- a turn or a rating with write-back wakes the write-back (`wake_writer`, `crates/app/src/library.rs:176-180`);
- an added folder queues its scan (`Runner::scan`);
- a removed folder stops the scans under it (`Runner::folder_removed`, with the path the write returned).

All three take locks the GUI takes today ([Inventory](#inventory)).

The Qt side gains the two layers of [Design](#design) in `pg_client::Grid`, the delayed dimming, and the notes of R2.
`Grid.bordered`, which is maintained incrementally (`crates/client/src/grid.rs:323-342`), is derived from both layers.

The window's close changes with it ([Shutdown](#shutdown)).

### Part 5, the worker items

The file, lock and log work that the GUI thread still does moves off it, onto Part 5's workers. See
[Not the catalog](#not-the-catalog-but-on-the-gui-thread--worker-part-5).

### Part 6, enforcement and visibility

- R8's thread check, in every build.
- R10's moves to `pg-model`, the removal of `reexport.rs`, and its check: no `pub` item of `pg_app` names a `pg-catalog`
  type.
- Part 1's rule needs no check of its own: a read connection refuses a write, and a write function needs a transaction.
  A test opens a read connection and asserts that a write on it fails.

## Shutdown

**Closing waits for pending commands.** The close itself sends commands: `onClosing` runs `shell.flush()`
(`desktop/qml/Main.qml:138`), which saves the session and the last file (`crates/client/src/controller.rs:509-535`), written
synchronously today (`desktop/src/shell.rs:496`).

- On a close request the window flushes once. If the pending layer is then not empty, it refuses the close
  (`close.accepted = false`, which the code does not use today), shows a "saving" state with a "quit anyway" choice, and
  closes itself when the last ack arrives or the user chooses to quit.
- The job and the write-back are still running then, so a late turn is written and a late add-folder gets its scan
  queued.
- An edit is lost at exit only when the user chooses it; a command dropped that way is logged.
- *Quits that skip `closing`.* The screenshot harness calls `QCoreApplication::quit` and `exit`
  (`cpp/screenshot.cpp:174`, `:186`, `:211`), and its settle check does not see pending commands (`screenshot.cpp:79-112`,
  `shell.rs:242-244`); the check gains "no command is pending". The debug build's live reload deletes the root
  (`cpp/live_reload.cpp:95`); commands already sent still run on the command thread, and that is accepted for a debug
  build.

Today `crates/desktop/src/lib.rs:212-214` runs `app::stop()`, then `drop(engine)` (which joins the image providers' pools),
then `app::shutdown()`. Two things order the new steps. The writer must outlive everything that writes through it. And
no thread may be left inside `write`: each stop below raises the stop that its thread's `write` calls carry, so a call
that is queued, or waiting on another process's lock, ends with `Stopped`.

1. Stop the command thread. Its queue is empty, or the user chose to quit and what is left is dropped and logged.
2. `app::stop()`, as today (`crates/app/src/app.rs:148-154`): the Yandex search stops first (up to 2 s), then the listing
   lane, then the library (the write-back, then the job). `Library::stop` holds the write-back's slot mutex while it
   joins the write-back (`crates/app/src/library.rs:166-171`); nothing else that takes that mutex is running by now, since
   the command thread has stopped.
3. `drop(engine)`: the providers finish, and their pools join (`crates/desktop/cpp/providers.cpp:131-136`). The providers
   only read, so they may run through steps 1 and 2.
4. Stop the point lane. It answers its queued requests with an error.
5. `app::shutdown` (`crates/app/src/app.rs:160-177`): the thumbnail cache closes (`thumbs.close()`, `:164`); the event
   forwarder is joined (`:166-170`); the preferences writer drains and closes (Part 5); the inbox closes
   (`crates/app/src/inbox.rs:454-463`). The inbox's writes carry its 5 s drain as their deadline (`SHUTDOWN_DRAIN`,
   `inbox.rs:79`), so it abandons what it cannot write in that time, as its own busy loop does today
   (`inbox.rs:852-892`).
6. The `Database` closes. The watcher stops first, so it calls nothing that has closed. The read connections are dropped.
   The writer, whose queue is empty by now, runs `wal_checkpoint(TRUNCATE)` and closes its connection last, so the last
   connection removes the WAL, as the keeper does today (`crates/app/src/catalogs.rs:53-61`). A job thread that was
   abandoned after its stop wait (`crates/library/src/runner.rs`, `stop`) is not inside `write`: its stop ended the call.

The facade returns an error at once once stop has begun, so no call made during shutdown can hang. The joins run on the main
thread after the window has closed, when no UI remains (see [Decisions](#decisions)).

## Inventory

### Already off the GUI thread

- **The `Reader`** (asynchronous, delivered on the GUI thread): `gallery/model.rs` `open_listing` (`Session::grid`),
  `filters.rs` `count_facets` (`Session::facets`), `folders.rs` `reload` (`Session::tree`), `failed_files.rs` `show`
  (`Session::failed_files`), `notices.rs` `copy` (`Session::copy_details`), and `shell.rs` `reveal` (`Session::reveal_folder`,
  which replaced `Catalogs::present_folder`).
- **Queued to the inbox's thread**: `app.rs` `post`, `resolved`, `resolved_part`; `notices.rs` (marks, `set_viewing`,
  `dismiss`, `dismiss_all`, `retry`, `poke`); `library.rs` `on_pass` (`poke` after the job's rounds). `Inbox::post` is not free
  of GUI work: it logs synchronously first (`crates/app/src/inbox.rs:341-343`), taking the inbox's `logged` mutex as it does
  (`inbox.rs:495`), which Part 5 moves.
- **Queued to the job or the write-back**: `library().wake_job()` in `notices.rs` `retry` (a closure the inbox thread runs
  once the retry is written, `crates/app/src/inbox.rs:408-412`, so not a GUI call), and `library().start` (`library.rs`
  `start`: it starts the job and write-back threads and queues two inbox clears; it opens no catalog itself).
- **In memory, stays on the GUI thread**: `prefs().get()` (`crates/app/src/store.rs:46`, a clone under the store's lock, which
  Part 5 keeps free of I/O); `library().running()` (returns the started library from a cell, no I/O). Three more take a
  mutex that no thread holds across I/O:
  - `inbox().subscribe()` takes the `first` mutex (`inbox.rs:448-450`), and the Yandex `search().subscribe()`
    (`desktop/src/yandex.rs:120`) takes its bus's mutex;
  - `inbox().idle()` takes `shared.work` (`inbox.rs:467`), which the inbox thread holds only around its condvar wait and its
    queue bookkeeping (`:776-784`, `:879-890`, `:914`);
  - `inbox().listing()` takes a separate `listing` mutex (`:443`), which the inbox thread holds only for a pointer swap
    (`:959`).

  The inbox thread holds none of them across SQL, so these stay.
- **In memory, but with a lock the GUI waits on, and a stop of the running pass**. Part 5 moves all of these:
  - `library().set_thumb_height` (`crates/app/src/library.rs:252`), which takes a mutex in `HeightCell::set` and wakes the
    job;
  - `library().save_crops` (`gallery/edits.rs:124`), which takes the job's queue mutex and wakes it
    (`crates/library/src/runner.rs:234-237`);
  - `library().wake_job()` in `on_pass`'s `turned` branch (`desktop/src/library.rs:142-144`): `request_stop` and `wake`
    (`runner.rs:240`, `:269-272`);
  - `Runner::scan` after `add_folder` and `rescan` (`runner.rs:208-217`): the queue mutex and `request_stop`;
  - `wake_writer` inside `library().turn` (`crates/app/src/library.rs:176-180`, `:339`): the write-back's slot mutex and `tend`,
    the same pair listed for `status()` below.

### Catalog reads on the GUI thread → point lane

| Site | Call | Reached from | Target |
| --- | --- | --- | --- |
| `gallery/state.rs` `with_details` | `reading().details` | `gallery/model.rs` `refresh_viewed` (from `set_viewing_holding` and `bump_revision`) and `refresh_derived` (`model.rs:264-272`); the `info` and `border_rejected` callers are Rust, not QML | point lane, into the QML property |
| `gallery/state.rs` `fresh_entry` | `reading().entry` | `Grid::arrive` (`crates/client/src/grid.rs:289`, for edited ids) and `refresh_turned` (`gallery/edits.rs`) | removed for `arrive` (the two layers and R2's notes, below); the point lane's re-read at `Turned` for `refresh_turned` |
| `gallery/state.rs` memo | two entries (`state.rs:129-130`) | `with_details` | replaced by the read-ahead (Part 3) |
| `animation.rs` `restart` | `catalogs().file_path` | the viewer starting an animation | point lane |
| `video_source.rs` `play` | `catalogs().file_path` | the viewer starting a video | point lane |
| `yandex.rs` `search_file` | `catalogs().file_path` | the Yandex panel's search of a file | point lane |
| `library.rs` `load_roots` | `library().roots()` | `start`, `add_path`, `remove_folder` | point lane for `start`; the command's ack after an add or a removal (R4) |
| `library.rs` `rescan` | the roots read (`crates/app/src/library.rs:327-329`) | `rescan` (`crates/desktop/src/library.rs:240`), before it queues the scans | point lane, then the library job queues the scans (R7) |
| `shell.rs` `view_names` | `views().all()` | the saved views' list, after each save and delete, and at startup (`shell.rs:48`) | the command's ack after a save or delete (R4); the startup worker's slot at startup (R9) |
| `shell.rs` `apply_view` | `views().view` | the views menu | point lane |
| `controller.rs` `Core::new` | `views().listing()`, `views().view`, `prefs().get()` | `Shell`'s construction during QML load, before the first frame | read on the startup worker and taken from the slot (R9) |

**The listing's `arrive`.** `Grid::arrive` reloads the edited ids (`crates/client/src/grid.rs:282-292`) through the synchronous
`fresh_entry`, for the ids the GUI has edited while the list loaded. The design needs no such reload. The listing's rows go to
the stored layer under R2 with the listing's revision. An edit made while the list loaded is either still in the pending
layer, which is drawn over the new rows, or was acked while the read was in flight. In that case R2's notes hold its
`Change`, and the entry is read again when the older listing lands.

### Catalog writes on the GUI thread → commands to the writer

| Site | Call | What it writes | Target |
| --- | --- | --- | --- |
| `gallery/edits.rs` `rotate` | `library().turn` | the `file_writes` turn row, then `wake_writer` (the write-back's slot mutex and `tend`); the image write happens on the write-back thread | command (R6: turns add up); the command thread wakes the write-back after the commit |
| `gallery/edits.rs` `set_rating` | `edit().rate` | `set_rating` (and its write-back) | command (R6: last value) |
| `gallery/edits.rs` `set_border_marked` | `edit().set_border_mark` | `border_marks` | command (R6: last value) |
| `shell.rs` `save_view`, `delete_view` | `views().save`, `views().delete` | `views` | command |
| `shell.rs` `execute` (`SaveSession`) | `views().save_listing` | `session` | command |
| `library.rs` `add_path` | `library().add_folder` | normalize and `check_folder`, then `add_root`, then `Runner::scan` | the probes as a probe task, then one command for `add_root`; the command thread queues the scan after the commit (R7) |
| `library.rs` `remove_folder` | `library().remove_folder` | a roots read in its own statement (`crates/app/src/library.rs:315-322`), then `remove_root` (one transaction; sets `missing = 1` over the folder, `crates/catalog/src/roots.rs:58-72`), then `Runner::folder_removed` with the root's path (the job's queue and busy mutexes, and a stop request) | one command, one transaction, without the outer read; the write returns the path, and the command thread calls `folder_removed` after the commit (R7) |

`add_path` reaches `check_folder` on the GUI thread today: `add_path` (`desktop/src/library.rs:211-212`) calls
`app::get().library().add_folder` (`crates/app/src/library.rs:298`), which calls `normalize_path`
(`crates/library/src/folders.rs:29-31`) and `check_folder`. On Windows `check_ntfs` (`crates/scan/src/refusal.rs:39-66`) makes the
volume lookup, the drive-type query, a handle open, the reparse-point test, the final-path call and the file-system query
(`crates/platform/src/dir.rs:117` onward). All of that is file or volume I/O, and it moves to a probe task (R7). Today
it runs inside the `with_catalog` closure, holding a pooled connection but no lock.

### Not the catalog, but on the GUI thread → worker (Part 5)

- **Preferences.** `prefs().update` (`settings.rs`, `shell.rs` `prefs_effect`) and the Yandex save (`desktop/src/yandex.rs:357`,
  which reaches `prefs.update` in `crates/app/src/yandex.rs:550`) write the preferences file on the GUI thread.
  `JsonStore::update` holds its lock across the serialize and the fsync (`crates/app/src/store.rs:51-55`), and `JsonStore::get`
  takes the same lock (`store.rs:46`), so a GUI `get()` waits for a save's fsync. The fix: `update` changes and serializes under
  the lock, then releases it. The file write goes to one ordered preferences writer (a thread of its own, not the catalog's
  writer) that always writes the newest snapshot, so queued saves cannot land out of order. It drains and closes in
  `app::shutdown` ([Shutdown](#shutdown)), so a save queued at exit is not lost. A failed save posts its notice from there.
- **`picker.rs` `start_for`**: `yandex().start_folder()` stats the saved folder (`is_dir`) and takes the search's state lock
  (`crates/app/src/yandex.rs:520-527`). `picker.rs:53` also canonicalizes (`crates/platform/src/picker.rs:44-50`). Target: a probe
  task.
- **`set_thumb_height`**: takes a mutex and stops a running pass (`crates/library/src/runner.rs:57-76`, which wakes the job
  after releasing its own mutex; the stop is `request_stop`, `crates/jobs/src/control.rs:117-121`, which takes the `paused`
  mutex). The GUI keeps its own copy of the height, because the layout uses it; a worker is told of each change.
  Target: a worker request.
- **`save_crops`, `wake_job` in `on_pass`, `Runner::scan`**: the same job mutexes and the same stop of the running pass
  (see [Already off the GUI thread](#already-off-the-gui-thread)). Target: worker requests. `Runner::scan` after an add
  and `Runner::folder_removed` after a removal run on the command thread, after the commit (Part 4).
- **The animation player's join.** Dropping a `Player` joins its thread (`crates/render/src/player.rs:55-65`). `restart`
  drops one on every change of an animated item (`desktop/src/animation.rs:54`), and that thread may be inside
  `std::fs::read`. Target: the old player is handed to a worker to drop.
- **The clipboard.** `setText` (`desktop/src/notices.rs:441`, `cpp/clipboard.cpp:12-13`) runs on the GUI thread, where Qt
  requires it. Whether it can wait on another process is not checked ([Open](#open)). Accepted.
- **The Yandex search.** `search().start` (`desktop/src/yandex.rs:102`, from `search_file` and `search_command_line`) takes
  the search's `shared` mutex, publishes and spawns a thread (`crates/app/src/yandex.rs:421-436`). `search().poll()` runs
  on every pump pass (`desktop/src/yandex.rs:263`) and takes the same mutex and the pictures' `take_dead`
  (`app/src/yandex.rs:485-489`). How long the search's workers hold `shared` is not checked ([Open](#open)). Target: the
  search pushes its state, as the library's status does below.
- **The log panel.** `pg_log::subscribe()` (`desktop/src/log_model.rs:112-113`) takes the logger's `ring` mutex and clones
  the kept records (`crates/log/src/lib.rs:299-305`). The file read is already on its own thread. Target: the subscribe
  moves to that thread.
- **`library().status()`** in `refresh_progress` and `refresh_writes` (`desktop/src/library.rs:166, 193`): takes the write-back's
  slot mutex (`crates/app/src/library.rs:211-213`), and `tend` restarts a dead write-back thread under it (`:125`). Target: the
  library pump's thread reads the status when an event arrives and hands it to the Qt pass with the folded changes. It is
  not a `Change`, which stays `Copy` and small. The GUI keeps the last status it was handed.
- **`set_paused`, `is_paused`**: take the job's pause and queue mutexes (`crates/library/src/runner.rs`, by symbol).
  Target: worker requests, with their state pushed back.
- **Logging.** Every `pg_log` call takes the ring lock (`crates/log/src/lib.rs:269`) and the logger's `writing` lock, and
  appends synchronously: it stats the file, checks that it exists, and opens it, for each line
  (`crates/log/src/lib.rs:240-249`, `crates/log/src/file.rs:53-63`). Target: a channel to a log thread, flushed on exit and on
  panic. A crash can lose the last lines, which the plan accepts for the GUI's own log calls. `Inbox::post`'s log line goes
  the same way.
- **Video.** `video_player.rs:14-16` calls `File::open` (`crates/platform/src/file.rs:164-166`, the Windows variant; the other
  variant is not checked). `cpp/video_source.cpp:47-60` then opens that handle as a `QFile` (`_open_osfhandle`,
  `QFile::open(fd)`), also on the GUI thread. `Player::start` (`animation.rs:68`) reads its file on its own thread
  (`crates/render/src/player.rs:72`), so it is not a GUI call. Target: the player's thread.
- **The native picker's modal dialog** (`crates/desktop/src/picker.rs:85`): the user waits on it, and its own event loop runs.
  Accepted. The non-native picker's folder listing (`picker.rs:102` and `:170`, Linux only) is a worker request.
- **Startup**: `app::open` itself, on the main thread before the GUI application exists (`crates/desktop/src/lib.rs:164`),
  which R9 moves to the startup worker. Also the `gpu_thread` join after QML load (`crates/desktop/src/lib.rs:194-197`), the pipewire probe (`lib.rs:113`),
  `Config::from_env` (`lib.rs:137`), and the log and panic-log setup (`pg_log::init`, `lib.rs:142`; `panic_log::install`,
  `lib.rs:146`), which open files. They run once, before the first frame, and are accepted (see the Goal). Their durations
  are in [Open](#open).
- **Shutdown**: the joins of [Shutdown](#shutdown), and `~Provider`'s wait (`crates/desktop/cpp/providers.cpp:131-136`). The joins
  are accepted, with the bounds given in [Decisions](#decisions).
- **Env-gated and harness writes**: `PG_RECORD`'s file creation (`crates/desktop/src/controller.rs:57`) and its synchronous
  write of the init record and of every controller update (`controller.rs:65-66`, `:76`, `:129`); the screenshot writes
  (the screenshot harness); and the debug build's live reload, which lists a directory and watches it
  (`cpp/live_reload.cpp:64-68`). Accepted: they run only when the environment or the build asks for them.

## Measurements

Two passes, both on 2026-11-13. Each probe ran on a copy of the live catalog (`catalog.sqlite`, 230,812 files, 2 roots, no
pending files), and called the same functions the services call. The live catalog was never opened. The probes were
temporary and are deleted, so these numbers cannot be rerun from the repo. Two temporary source changes were made and restored
with `git checkout`: the import's flush was timed, and `wal_autocheckpoint = 0` was added to `SETTINGS` for one experiment.
The library job and writer were never started, so no image file was written. Debug build unless marked release.

The first pass's import flush table (one 500-file batch) is superseded by the second pass and is not kept. Its debug
figures agree with the second pass; its release batch total (1,833.6 ms) is lower than the second pass's release flushes
(2,066 to 2,386 ms), and the difference is not explained.

### Idle GUI-thread calls (debug, ms)

| Call | p50 | p95 | max | n |
| --- | --- | --- | --- | --- |
| `reading().details` | 0.118 | 0.210 | 0.390 | 300 |
| `reading().entry` | 0.082 | 0.129 | 0.232 | 300 |
| `catalogs().file_path` | 0.048 | 0.068 | 0.136 | 300 |
| `views().all` | 0.013 | 0.020 | 0.160 | 50 |
| `views().listing` | 0.018 | 0.026 | 0.156 | 50 |
| `views().view` | 0.014 | 0.018 | 0.025 | 50 |
| `library().roots` | 0.013 | 0.019 | 0.078 | 50 |
| `edit().rate` (write) | 0.276 | 0.433 | 4.406 | 60 |
| `edit().set_border_mark` (write) | 0.060 | 0.130 | 0.179 | 60 |
| `library().turn` (write) | 0.182 | 0.308 | 736.847 | 60 |

The first `details` call on a cold pool took 3.041 ms.

### With another connection holding the write lock (first pass, ms)

| Case | Result |
| --- | --- |
| 200 reads (`details`, `entry`) during a 2 s hold | p50 0.076, p95 0.103, max 0.163. No waiting. |
| `edit().rate` during a 2 s hold | 2,090 ms, succeeded |
| `edit().rate` during a 7 s hold | 5,327 ms, failed with "database is locked" |

### Turn stall (second pass; 3,000 turns on image files, debug)

| Setting | Runs | Stalls of 100 ms or more | Slowest (ms) | WAL before the stall |
| --- | --- | --- | --- | --- |
| Auto-checkpoint on (default) | 4 | one each, always at sample 142 | 838.7, 548.7, 553.9, 512.7 | 4,111,792 bytes (about 1,000 frames) |
| Auto-checkpoint off (temporary) | 3 | none | 3.2 to 5.0 | none; the WAL reached 90,030,272 bytes |

Only one stall of 100 ms or more was seen per run, at the first 1000-frame checkpoint. The later checkpoints (about 21 in a
3,000-turn run, at roughly 7 frames per turn) were not timed separately. So the data show that this checkpoint stalled, and
the stall disappears without it. They do not show that every checkpoint stalls ([Open](#open)). The typical turn took 0.16 to
0.22 ms (p50) in both settings.

### Import flush, release (second pass; 2,000 pending files, four batches of 500)

| Batch | Flush (ms) | Pairing inside the flush (ms) | Pairing share | SQL share (flush minus pairing, ms) |
| --- | --- | --- | --- | --- |
| 1 | 2,333.5 | 1,775.1 | 76% | 558.4 |
| 2 | 2,386.2 | 2,193.3 | 92% | 192.9 |
| 3 | 2,066.1 | 1,875.3 | 91% | 190.8 |
| 4 | 2,235.1 | 2,041.6 | 91% | 193.5 |

The import took 15.15 s, with 854 pairs and no failed file. With a rating loop during the import, the flushes were 2,035 to
2,467 ms and the pairing 1,821 to 2,142 ms. Pairing is in-memory work (`crates/dedup/src/pairer.rs`), so the pairing share is
matching time, not SQL time.

### Import flush, debug (second pass; 500 pending files, one batch)

Flush 12,363.4 ms; pairing 12,159.3 ms (98%). The import took 35.8 s.

### Rating during the import (second pass)

`set_rating` with write-back on, one call per loop iteration on a second connection. The loop slept 20 ms between calls.
The run time divided by the count is about 52 ms (debug) and 66 ms (release), but that average includes the time the loop
spent blocked on the import's lock (about 15 s of the debug run and about 8 s of the release run). An unblocked iteration
took about 30 ms: the call plus a 20 ms sleep at the Windows timer's granularity.

| Build | Import | Ratings ok | Failed | Wait p50 / p95 / max (ms) |
| --- | --- | --- | --- | --- |
| Debug, 500 files | 35.8 s | 688 | 2, both "database is locked" after 5.3 s | 0.41 / 1.18 / 1,582 |
| Release, 2,000 files | 15.1 s | 228 | 0 | 0.49 / 2.26 / 2,345 |

### Bulk writes (second pass; debug, three runs each, a fresh copy each time)

| Operation | Rows | Time (ms) |
| --- | --- | --- |
| `remove_root`, the 189,519-file root | 189,519 hidden | 3,622.8, 2,303.8, 2,373.4 |
| `remove_root`, the 41,293-file root | 41,293 hidden | 992.8, 910.6, 897.1 |
| `add_folder`, synchronous part (normalize, NTFS check, `add_root`; the scan is not run) | one folder | 1.8, 2.3, 1.5 |

### Cold reads (second pass; debug, 300 random `file_path` reads each)

| Phase | p50 (ms) | p95 (ms) | max (ms) |
| --- | --- | --- | --- |
| After reading 2.9 GB of other data | 0.027 | 0.030 | 0.123 |
| Fresh connection, same ids | 0.027 | 0.049 | 0.126 |
| Same connection, same ids | 0.020 | 0.022 | 0.033 |

The machine has 64 GB of RAM with 21.7 GB free, so reading 2.9 GB did not evict the OS cache. The first phase is therefore not a
cold-cache measurement, and the cold cost is not established ([Open](#open)). The `Reader` clears its page cache after each job
(`crates/app/src/reader.rs:183`), so a point lane built on the `Reader` would pay the fresh-connection cost on each request. The
point lane keeps its page cache, so the warm figures apply to it.

### What the numbers show

- Reads don't wait on a writer: the 200 reads during the 2 s hold had a maximum of 0.16 ms.
- A GUI write waits for the whole hold. A 2 s hold made the rating wait 2.09 s. A 7 s hold made it fail at 5.3 s, so the edit was
  lost. That is the 5 s busy timeout.
- Each import batch holds the write lock for its whole flush: 2.0 to 2.5 s per 500 files in release, and 12.4 s in debug. In
  debug, two ratings failed with "database is locked" after 5.3 s. In release, none failed; the longest wait was 2.35 s.
- The start-state read (`Core::new`) runs before the first frame. It is a read, so it does not wait on a writer.

## Open

What is still open. Nothing here is built.

- **Checkpoints, counted. This gates Part 2b.** Count the checkpoints in a 3,000-turn run, and time each one, to show whether
  every 1000-frame checkpoint stalls or only the first. If only the first, find what that one pays for.
- **How long the job's transactions hold the writer.** The scan batch (10,000 entries, or a whole larger folder), the
  borders batch, the pairing tiers and `mark_missing`, and the preparation's marks run by another process are not
  measured (Part 1). They decide how long an edit can stay pending. A scan batch that is a whole folder has no cap.
- **Removing a folder in shorter transactions.** With one writer, the order of the removal's steps is the queue's order.
  Whether that makes a split safe against a scan or a re-add between the steps is not designed (R7).
- **A neighbour that changed during the import's matching.** Whether a pair with an already-indexed file that changed
  between the matching and the commit should be re-checked inside the write, and whether today's import has the same
  window, is not traced (Part 2a).
- **The inbox's steps as writes.** Each step stays its own transaction, and its busy loop becomes the `write` call's
  deadline. How its `Store` trait (`crates/app/src/inbox.rs:759`), which has the busy loop built in, changes is not
  designed.
- **Cold-cache reads: not established.** Reading 2.9 GB did not evict the OS cache on this machine (64 GB of RAM, 21.7 GB free).
  Evicting it needs far more reads than a probe should do, or a reboot. Until then, the point lane's cost is the warm figure,
  about 0.02 to 0.03 ms for a `file_path` read.
- **The dimming delay.** The 200 ms is chosen. How long an ack takes to reach the Qt thread is not measured (R4).
- **Rescan's scans.** `rescan` was not measured with the scans run: with the library not started, it only reads the roots.
- **Startup and shutdown durations.** The `gpu_thread` join, the pipewire probe, the log setup, `app::open` and the start
  state's reads on the worker, `~Provider`'s wait for the pool (`crates/desktop/cpp/providers.cpp:135`), the Yandex stop
  (`STOP_WAIT`, 2 s, `crates/app/src/yandex.rs:102`), the job's stop (`JOB_STOP_WAIT`, 5 s, `crates/app/src/library.rs:32`),
  and the catalog close (`wal_checkpoint(TRUNCATE)`). Their durations decide whether the shutdown bound in
  [Decisions](#decisions) is typical, and how long the placeholder shows. Not measured: they need the app launched and
  closed with timing.
- **Shutdown waits outside the bound.** `~Provider`'s `waitForDone` has no bound (`crates/desktop/cpp/providers.cpp:135`),
  and a Yandex pool job can be in a network fetch when it is called (`crates/desktop/src/providers.rs:99`).
  The write-back's join (`crates/app/src/library.rs:166-171`) has no bound either: its `write` calls end on stop, but an
  image write in progress does not. `thumbs.close()` (`crates/app/src/app.rs:164`), the forwarder join, the preferences
  writer's drain, the command thread's and the watcher's joins, and the writer's `TRUNCATE` are not in the bound, and are
  not measured.
- **Pairing shares.** The second pass's pairing shares come from a deleted probe and are not re-checked. The batch-matching cost
  of Part 2a is not measured.
- **Unchecked sites.** How long the Yandex search's workers hold `shared`; whether the clipboard's `setText` can wait on
  another process; whether the import pipeline delivers a batch in item order (Part 2a needs only the batch's own order,
  which it has).
- **Designed, not built:** Parts 1 to 6, with their tests as given in each part.
- **Verified against the code:**
  - the write-back has the file id at `Turned` (Part 3);
  - the rebuild ends with `wal_checkpoint(TRUNCATE)` and a close (`rebuild.rs:119-120`), and needs the exclusive hold;
  - the inbox's `idle()` and `listing()` take different mutexes, and the inbox thread holds neither across SQL
    ([Inventory](#inventory));
  - rusqlite's `busy_handler` and `wal_hook` take closures at the locked revision (`src/busy.rs:55-57`,
    `src/hooks/mod.rs:389-391`), and `total_changes()` is exposed (`src/lib.rs:1064`);
  - the bridge objects are QML elements built through `Default`, so the app cannot be passed to them (R9);
  - the keeper is only the prepared connection held open, so the writer's connection can take its place;
  - no read path writes: none uses a temp table, `ANALYZE` or a lazy insert, and `Catalog::open` writes nothing once the
    file is in WAL mode. So `query_only` breaks no read;
  - no transaction outlives one flush, and nothing reads its own uncommitted writes across transactions, so each can be
    one `write` call;
  - the thumbnail pass's only catalog writes are the failure ledger's;
  - an entry's `rotation` is `files.rotation` and its `pending_turns` is the queued `file_writes` value;
  - `Player::start` reads its file on its own thread (`crates/render/src/player.rs:72`), and `requestImageResponse` runs on
    Qt's pixmap-reader thread (`qquickpixmapcache.cpp:1000-1003` in the Qt source);
  - there is a single window, and no window geometry is persisted.

## Decisions

Decided, with the reason:

- **One writer per process** (Design, Part 1). Several writers on one lock was the cause of the lost rating and of the
  waits behind the import. With one writer, a write inside the process cannot fail busy, a new edit waits for what is in
  progress and not for a queue of job writes, and the order of writes is a queue's order. Whether an earlier review
  considered and rejected this is not known: their findings were not kept.
- **The writer in `pg-catalog` takes opaque closures; commands, coalescing, acks and what follows a commit belong to a
  command thread in `pg_app`** (Design). `pg-catalog` cannot know the bus, the job or the write-back. And what follows a
  commit takes locks: on the writer's thread it could deadlock the shutdown, where the main thread holds the write-back's
  slot mutex while the write-back waits on the writer.
- **Every `write` call carries its caller's stop** (Design, R5). A thread told to stop is never left inside `write`, so
  each stop in the shutdown returns, whatever another process holds.
- **Every other connection is `query_only`, closed by drop, and a write function takes a transaction** (Part 1). The rule
  is enforced by SQLite and by the types, so it needs no check that greps the source. Closing by drop matters because
  `query_only` does not stop a checkpoint.
- **The `Database` is the only way to open a catalog, and it performs the thumbnail cache's attach** (Design). No
  connection on the file is then unknown to it, and the app, the command-line tool and the tests share one behaviour.
- **The preparation's writes run on the writer** (Part 1). They change values a running window shows, so they must bump
  the revision.
- **The scan's lease is released by a call, and its heartbeat rolls a batch back when the lease is lost** (Part 1). A
  write in a destructor could block during an unwind. A job write can now wait long enough to lose the lease.
- **A closure may borrow its caller's state, through one documented `unsafe` block** (Design). The flushes borrow
  pass-local state today, and the caller blocks until the closure has run. The alternative is to restructure every
  flush to own its data.
- **A panic in a closure is resumed on the caller's thread** (Design). The job and the write-back each have their own
  policy for a panic, and both keep it.
- **The writer serves edits first, then the write-back and the inbox, then the job, and never interrupts** (Design). An
  edit is what the user is watching; a job write is what can wait.
- **A transaction holds only SQL, and decides inside itself** (Design, Part 2a). Anything computed inside it keeps every
  other write waiting, and anything read before it, on another connection, may be stale.
- **One catalog-wide revision, bumped once at commit, and only when the transaction changed something** (Part 1). A sequence
  from the GUI's commands alone cannot order the import, scan and removal writes, and a read must be able to say which
  writes it includes. An unconditional bump would turn empty commits into writes. Migrations bump explicitly, because
  `total_changes` does not count DDL.
- **Part 1 includes the command thread, with the GUI still waiting for each ack** (Build order). Part 1 makes the pool
  read-only, so the GUI's writes need their new path at once. The 5 s wait keeps today's behaviour until Part 4.
- **Reads never wait; the point lane never drops a request** (R1, Part 3). A read that waited on a pending write would tie the
  point lane to the writer. A `Reader`-style replacement would drop results.
- **The GUI holds stored values and pending commands as two layers, in `pg_client::Grid`** (Design, R1, R4). A read never
  has to be dropped or merged because an edit is pending, and a failed command needs no read to undo it. They live in
  the grid because the layout reads the entries there.
- **The stored layer only moves forward, and a change noted while a read was in flight is read again** (R2). A revision
  check alone loses a file that changed while a listing containing it was loading. The notes exist only while a read is
  in flight, and they ask for a read; they never refuse a result. An earlier draft rejected any such note; that was
  wrong.
- **An ack carries what the catalog holds, read inside the command's transaction** (R4). The stored layer is refreshed in
  one snapshot at the ack's revision.
- **A pending turn does not turn the picture; the image key follows the stored layer** (R4). A key ahead of the catalog
  makes the providers serve, and Qt cache, unturned pixels under the turned key. The picture turns at the ack, as it
  does today after the queue write commits.
- **Dimming starts after 200 ms pending** (R4). An ack normally lands on the next loop turn, and dimming at once would
  flicker on every edit.
- **The point lane has its own connection and keeps its page cache**, opened at startup, so the GUI never opens one.
- **Coalescing is per file and operation, on the command thread** (R6). Per file alone drops turns, because a turn is a
  delta.
- **The pending layer is capped at 10,000 commands, on the Qt thread** (R5). The GUI is the only place that knows the
  count when a command is made. The value is chosen, not measured.
- **The import computes its pairs outside the write transaction, stores only the pairs of stored files, and inserts into the
  index only after commit** (Part 2a). The reason is the hold: pairing was 76 to 92% of it, and it would now be the
  writer's. Today's import already skips pairing for a rejected file and has no index rollback, so this fixes no
  correctness fault. A failed commit leaves the index unchanged.
- **The writer waits on another process through a busy handler that gives up on the call's stop** (R5). No write is lost
  to a timeout, and no writer has a busy policy of its own.
- **The busy state shows after 10 s** (R5). It is a QML property, not a database notice, since a database notice is itself
  held by the same lock.
- **A checkpoint, if it must leave the commit path, is the writer's lowest-priority work, not a thread of its own**
  (Part 2b). It runs when no edit is waiting, including when the writer is idle. It bounds the WAL no better than the
  automatic checkpoint does: a long-held reader pins both. Part 2b is built only if the counted measurement shows that
  checkpoints stall as a rule.
- **The checkpoint trigger is the WAL frame count, through the WAL hook, not the file size** (Part 2b). The file keeps its size
  after a checkpoint, so a size test would fire on every commit.
- **Removing a folder is one transaction, and returns the removed root's path** (R7). The measured hold is 2.3 to 3.6 s
  (debug, one machine). Every write queued during it waits and none fails. The path is what stops the scans under the
  folder. Splitting the transaction is open.
- **An add's probes run as their own task, before the command is sent** (R7). On the writer, the command thread or any
  ordered queue, a dead drive would hold everything behind it.
- **A rescan is not a catalog write** (R7). It reads the roots and queues the scans in the library job.
- **Another process's write is a stored revision this process has not seen, checked by the writer in every transaction
  and by a watcher once a second** (R3a). `data_version` also moves on this process's own commits. The watcher alone
  misses a foreign write that an own commit follows before it looks; the writer alone misses one while it is idle.
- **The inbox keeps its own `data_version` comparison** (R3a). It also tells the inbox of this process's ledger writes.
- **The command-line tool keeps writing the catalog itself.** The alternative is one process owning the file, with the tool
  sending its work to a running window. That would remove R3a and cross-process waits, at the cost of a protocol between
  the two. Two processes on one WAL database is a supported arrangement, so it is not taken.
- **A lagging subscriber, a foreign write and the pump's first pass all refresh one fixed list** (R3). The list includes
  the folder tree, the facets and the status lines, which a re-read of the grid alone would leave stale.
- **The events that follow a commit carry its revision; the job's batches publish nothing per batch** (Part 3). `Scanned`
  and `Imported` already refresh what a scan or an import changes, once per pass.
- **Every pump waits with `recv()`, drains with `take()`, and reads the lag count** (Part 3). `take()` is the only reader of
  the count, and no pump reads it today.
- **Per-file changes reach the grid in Rust, not through the QML signal** (Part 3). The signal chain carries no payload
  and is recorded for replay.
- **No second cache of the catalog with its own invalidation** (Qt-side values). A stored value is what one read or one ack
  returned. The details memo is replaced by the read-ahead.
- **Preferences: the value is read from memory, and the file is written by the preferences writer, which drains at
  shutdown** (Part 5). A save no longer blocks the GUI's reads, and queued saves cannot land out of order.
- **The log is written by a log thread, flushed on exit and on panic** (Part 5). Every log call opens the file today. A
  crash can lose the last lines, which is accepted for the GUI's own calls.
- **Providers use `Images` and the catalog pool on their own threads, not the point lane** (R8). The point lane is not a
  bottleneck for them.
- **Blocking calls from the GUI thread are checked in every build; a violation logs rather than panics** (R8). A panic across the
  bridge aborts the process. The thread role lives in `pg-platform`, so `write` can check it below `pg_app`.
- **The front end sees `pg_app`'s messages and `pg-model`'s types, and nothing of `pg-catalog`** (R10). The commands and
  acks cross the boundary, so their payloads must be types the front end may name.
- **The window opens with a loading placeholder; the app is opened and the start state read on a startup worker** (R9).
  This is a cost the goal forces. The start state is read on the worker because the controller is built from it
  synchronously and has no loading state.
- **The app stays a process-wide value, and a `Loader` creates the main content on Ready** (R9). QML constructs the bridge
  objects through `Default`, so they cannot be handed the app. The `Loader` makes the order a fact of construction: no
  bridge object that reaches the app exists before the app does.
- **A failed open stays fatal: the same native box, then exit** (R9). There is no app, and so no inbox, to carry a notice.
- **Closing waits for pending commands, with a "quit anyway" choice** ([Shutdown](#shutdown)). An edit is lost at exit only
  when the user chooses it. A drain with a deadline after the window has gone would lose edits silently. The close's own
  session save is one of those commands.
- **The shutdown joins run on the main thread after the window has closed.** No UI remains to block. The bounds for the listed
  waits are: the Yandex stop 2 s, the job stop 5 s, and the inbox drain 5 s. Together they are about 12 s. Excluded, and
  unmeasured ([Open](#open)): `~Provider`'s unbounded wait, the write-back's join, `thumbs.close()`, the forwarder, the
  preferences writer's drain, the command thread's and the watcher's joins, and the writer's `TRUNCATE`. The bound is
  accepted for the listed waits only.
- **The read-ahead covers the viewer's current item plus its previous and next** (Design, Part 3). Arrow navigation in the
  viewer needs the neighbours ready. A read measured at most 0.39 ms.
- **A pending edit is shown dimmed, with its pending state exposed to assistive technology** (R4). Nothing looks done before the
  ack. Dimming does that without adding a marker to the image.
