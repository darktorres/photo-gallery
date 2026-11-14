# GUI thread plan: the window's catalog work off the Qt thread

Status (2026-11-14): a plan, not started. This plan has changed no product code; other work is in progress in the tree.
Fourteen adversarial reviews have been applied. The design was reworked around one writer per process after the fifth,
and the sixth to fourteenth reviewed that rework. None since the eighth found a blocker. The eighth to thirteenth each
still found a fault in the close and session-end handling, which has been rewritten twice; the fourteenth found only
wording. The reviews' findings are not kept here. Claims that depend on code not yet
checked are listed under [Open](#open).

Seeded by [NOTIFICATIONS_FIX_PLAN.md](NOTIFICATIONS_FIX_PLAN.md) (item C2). The site lists come from grepping
`crates/desktop/src` for `app::get()` and from reading the call graph; the grep missed sites, so [Inventory](#inventory)
lists both. Sites are named by symbol. Line numbers drift: re-read each site by symbol before acting on it.

Words used with one meaning in the design text:

- **the writer**: the catalog's one writing thread. The thread that writes image files is *the write-back*; the one that
  writes the preferences file is *the preferences writer*.
- **command**: an edit the GUI sends to the command thread. The controller's own `Command` enum (`crates/client`) is named
  in full. A call to a Part 5 thread is a *request*.
- **pending**: an edit the user made that is not yet acked or failed. An added folder is pending from the click, before
  its command is sent. The catalog's `files.pending` (the "pending files" of the Measurements)
  and an entry's `pending_turns` (the turn queued in `file_writes`) are named in full.
- **revision**: the catalog's revision of Part 1.
- **worker**: any thread that is not the GUI thread and not the writer.

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
- the clipboard, which Qt requires on the GUI thread;
- a mutex that no thread holds across I/O or a wait (the inbox's `first`, `work` and `listing`, the Yandex bus's);
- the wait on the command thread and the preferences writer when the operating system ends the session
  ([Shutdown](#shutdown));
- env-gated, harness and debug-build work (`PG_RECORD`, the screenshot harness, live reload), which runs only when the
  environment or the build asks for it.

Each accepted item is listed in [Inventory](#inventory), except the session-end wait, which is in [Shutdown](#shutdown).

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

- **`Database`** (`pg-catalog`). The one way to open a catalog file for the app, the command-line tool and the tests.
  `Database::open` does what `Catalog::prepare` does today, in this order: the opening thread takes the preparer's
  lock, rebuilds or migrates, makes the checks (`crates/catalog/src/core.rs:69-114`) and registers the SQL functions on
  the connection (`core.rs:120`); it starts the writer's thread and moves the prepared connection to it; it makes the
  preparation's marks through the writer, as `write` calls from the opening thread (Part 1); it drops the exclusive
  hold if it has one and takes the shared one, with the preparer's lock still held (`core.rs:123-124`); and it starts the
  watcher when it is given a function to report to. The `Database` owns the shared hold, not the connection. It hands out read connections and the writer's handle. `Catalog::open` and `prepare` stop being
  public. It is a value the app owns and passes, not a `static`. Everything that takes the catalog's path today takes
  the `Database`: `Runner::start` (`crates/library/src/runner.rs`), the write-back's `Writer::start`
  (`crates/writeback/src/lib.rs:222`, restarted in `tend`), `pg_thumbs::run` (`crates/thumbs/src/lib.rs:173`) and
  `Inbox::start` (`crates/app/src/app.rs:66`).
  - *One connection it does not own.* The thumbnail cache opens its own SQLite file and attaches the catalog to it, on a
    `query_only` connection that it opens and drops for each call (`crates/thumbcache/src/reader.rs:24-28`, `:62`, called
    from `crates/thumbs/src/lib.rs:180`). `thumbcache` and `catalog` cannot see each other in the crate layers, so the
    `Database` cannot perform that attach. It stays as it is, and is named here as the exception. It only reads.
- **Read connections** (`pg-catalog`). Every connection but the writer's is opened with `PRAGMA query_only = ON`, so SQLite
  refuses a write on it. `Catalog.conn` is `pub(crate)`, so nothing outside the crate can switch the pragma off. A read
  connection is closed by dropping it, never through `Catalog::close`: `query_only` does not block a checkpoint, and
  `close` runs `wal_checkpoint(TRUNCATE)` (`crates/catalog/src/core.rs:52-53`). Each read connection is owned by the thread
  that uses it (`runner.rs:349`, `crates/writeback/src/lib.rs:303`, `crates/app/src/inbox.rs:742`, `reader.rs:152`).
- **The writer** (`pg-catalog`). One thread with the one writable connection, held for the life of the process. It takes
  the keeper's place: the keeper is only the prepared connection held open (`crates/app/src/app.rs:62`). The order at
  open matters. When it is the sole opener, `prepare` holds the file exclusively until after its marks
  (`crates/catalog/src/core.rs:92`, `:115-123`),
  and every connection takes its own shared hold when it opens (`core.rs:136`), which would wait 5 s on that exclusive
  hold and fail. So the prepared connection itself is moved to the writer's thread, and the watcher's and every read
  connection open only after the open has changed its hold from exclusive to shared. Its one entry is
  `write(class, stop, |tx| ...)`: the caller blocks until the transaction commits, and gets its result and revision.
  - *Every transaction begins `IMMEDIATE`*, as `write_transaction` does today (`crates/catalog/src/transaction.rs:13-16`).
  - *Three classes*, served in this order, first in first out inside each: **edits** (the command thread), **service** (the
    write-back's and the inbox's small writes), **job** (the job's passes, the preparation's writes, the command-line
    tool's, the fixtures'). Edits and service writes are small and few, so they do not starve the job. The class is looked
    at again on every retry against another process's lock: a job write that is retrying has not begun, so an edit that
    arrives meanwhile goes first.
  - *Nothing is interrupted.* A new edit waits for what is in progress (one transaction or one checkpoint) and for the
    edits ahead of it, not for a queue of job writes. How long one transaction can be is in Part 1.
  - *Every call carries its caller's stop*, a `Stop` from `pg-platform`: a predicate that is polled. It can be a flag, a
    deadline, a flag with a deadline that starts when the flag is raised, or never. `pg-platform` has no such type today.
    It ends a call only while the call is **waiting**: queued behind others, or at the head while the writer waits on another process's
    lock. Such a call returns `Stopped`. A call that finds the writer free runs, and a closure that has begun runs to
    its commit. After the `Database` has closed, every call returns `Stopped`.
  - *Withdrawing a waiting call.* The caller and the writer meet on the queue's lock: either the writer has taken the
    call, and the caller waits for it to finish, or the caller removes it. So a borrowed closure is never left with the
    writer after its caller has returned.
  - *A waiting caller polls.* Whoever raises a stop knows only its own condvar (`Control::request_stop`,
    `crates/jobs/src/control.rs:117-121`; the service's quit, `crates/jobs/src/service.rs:111-116`; the inbox's close,
    `crates/app/src/inbox.rs:455-456`), not the writer's queue. So a waiting caller waits in short slices and checks its
    stop between them, as `checkpoint_within` does (`control.rs:192-208`). The busy handler checks the current call's stop
    the same way; it reaches it through a slot, since the handler itself is `Send + 'static`. When no call is current
    (the close's `TRUNCATE`), the handler gives up after 5 s, which is today's timeout.
  - *The stops.* A stop that ends a write at once would drop what today gets 5 s to commit, so the job's and the
    write-back's are a flag with a grace:
    - the command thread's: its own stop, with no grace. By then its queue is empty, or the quit did not wait for
      it (Shutdown, step 1). In Part
      1's interim each command also carries a 5 s deadline;
    - the job's: the new quit flag (below), with 4 s from when it is raised. That lets the final flush and the lease's
      release commit when the writer is only briefly busy. It is shorter than the 5 s the runner waits for the job
      (`JOB_STOP_WAIT`), so a write that gives up returns before the runner does, and the job is not reported abandoned
      for it;
    - the write-back's: its service's quit, with the same 4 s, so a `write_done` after a finished copy is not dropped;
    - *where the raise time lives.* These are two flags. The job's is `Control`'s new quit, since the passes receive only
      `&Control` (`crates/library/src/runner.rs`, the pass calls). The write-back's is its `Service`'s quit, which it
      reaches through the `Waker` (`crates/writeback/src/lib.rs:359`) and which keeps only a flag today
      (`crates/jobs/src/service.rs:111-116`). Both become one `pg-platform` type, a flag that records when it was raised,
      which `Control` and `Service` each hold, so a `Stop` can read it without `pg_jobs`;
    - the inbox's: its close, with the 5 s drain (`crates/app/src/inbox.rs:79`), which `report_lost` (`inbox.rs:901`) uses
      too;
    - `Database::open`'s: the one its caller passes (Part 1, R9);
    - the command-line tool's and the tests': never.
  - *The job's stop is a new quit flag, not `Control::stop`.* `Control::stop` is the job's preemption flag: every new
    request raises it (`crates/library/src/runner.rs`, `request_stop`), a round clears it, and its contract is "it commits
    what it has" (`crates/jobs/src/control.rs:116`). The passes write after it is raised: the final flush
    (`crates/jobs/src/pipeline.rs:211`, `:266-268`), the scan's stop batch (`crates/scan/src/lib.rs:580-582`), the thumbs
    ledger (`crates/thumbs/src/lib.rs:338-339`), the pairing, `mark_missing` and `finish_scan` (`scan/src/lib.rs:536-540`),
    and the lease's release. Used as the write stop it would drop those. `Control` gains a quit that only `Runner::stop`
    sets, and the job's writes carry that.
  - *No write from the writer's own thread.* A closure that calls `write` would wait on itself. The call checks the
    thread's role and returns an error.
  - *A panic in a closure.* The transaction's drop rolls it back, and the writer keeps its connection. The panic is
    resumed on the caller's thread, where the caller's own handling applies: the job catches a pass's panic, and the
    write-back catches a panic inside a job as that job's failure (`panics::guard`, `crates/writeback/src/lib.rs:411`).
    The panic hook asks a thread-local whether the panic will be caught (`CAUGHT`, `crates/jobs/src/panics.rs:11-14`), and
    shows the fatal box when it is not set (`crates/desktop/src/panic_log.rs:21-33`). The hook runs on the thread that
    panics, which is the writer's, and resuming the panic on the caller does not run it again. So the marker and
    what reads and sets it, `catch` and `caught()` (`crates/jobs/src/panics.rs:20-28`), move to `pg-platform`, with one
    new setter for the writer. `catch` uses only the standard library; `guard`, `message` and the rest stay in `pg_jobs`
    and call it. `pg_jobs` may not re-export it (`tools/boundaries.py`), so its six outside callers are repointed in
    Part 1 (two in `crates/app/src/yandex.rs`, one each in `yandex/images.rs`, `reader.rs`, `crates/library/src/runner.rs`, and the hook's
    reader, `pg_app::reexport::panic_caught`, `crates/desktop/src/panic_log.rs:21`, whose line in `reexport.rs` goes). Each `write` call carries its caller's marker value (`caught()`, `panics.rs:28`), and the writer sets
    its own marker to that value around the closure. A caller that catches gets no box; a caller that does not (the
    inbox thread, the write-back outside its job guard, and until Part 3 `Database::open`'s caller) still gets it, as
    today. From Part 3 the startup worker catches (R9), so its marker says so, the hook shows no box, and the failed-open
    path shows the one box. The box is
    shown on the thread that panicked (`panic_log.rs:25-32`), which is now the writer's, so every write waits until it is
    dismissed, and the log line names the writer's thread. The process does not end there: the
    hook returns after the box (`panic_log.rs:19-34`), the caller's thread dies with the resumed panic, and the writer
    goes on. So the writer is paused while the box is up. That is accepted.
  - *The closure may borrow.* The caller blocks until the closure has run or been withdrawn, so the hand-off is scoped,
    like `std::thread::scope`. The flushes borrow pass-local state today (`crates/import/src/lib.rs:50-89`,
    `crates/borders/src/lib.rs:60-117`, `crates/catalog/src/pair.rs:96-229`). The lifetimes of the closure and of the stop
    are erased in one documented `unsafe` block in `pg-catalog`; `write` still requires `F: Send` and `R: Send`, so a
    read connection cannot be captured.
- **A transaction holds only SQL.** Whatever a write needs computed is computed by its caller first. Part 2a is the first
  case: the import's pairing.
- **A write decides inside its transaction.** A worker now reads on one connection and writes on another, so what it read
  may be stale by the time its write runs. A closure re-reads, inside the transaction, whatever its write depends on, as
  `store_analysis` does with `IS_CURRENT`.
- **The thread role** (`pg-platform`). A thread-local with three values: GUI, writer, worker (the default). The Qt main
  thread and the writer set theirs at start. It lives in a crate `pg-catalog` may use, so `write` can check it (R8).
- **Command thread** (`pg_app`). One thread that owns the GUI's commands. It coalesces them (R6), runs each as a
  `write(edits, ...)`, and then, on its own thread, does what follows a commit: it replies with the ack, wakes the
  write-back, queues a scan, or tells the job a folder is gone (Part 1). None of that runs on the writer. A panic in a
  command is caught there: that command's ack is a failure, and the thread goes on. `Command` and `Ack` carry `pg-model`
  types only (R10).
- **Point lane** (`pg_app`). A FIFO executor with its own thread and its own read connection, opened at startup. It serves
  details, entry, `file_path`, views and roots. It never drops a request and never waits on the writer (a WAL read). It
  keeps its connection's page cache between requests. After stop it answers each queued request with an error.
- **Listing lane** (`pg_app`). The existing `Reader`, unchanged in behaviour (`crates/app/src/reader.rs:209-238`: the newest
  request per session and kind replaces the waiting one, and cancels the running one of the same kind). Its connection
  (`reader.rs:172`) becomes a read connection. Each result gains the revision of its snapshot.
- **Read-ahead and the selection.** Details are point-lane values, not derived from the grid. The GUI keeps four slots:
  the viewer's open item, its previous and its next (so arrow navigation finds the neighbours ready), and the grid's
  selection, whose line `refresh_derived` renders (`gallery/model.rs:264-272`). Each slot holds one result and its
  revision. They replace the two-entry details memo (`gallery/state.rs:56`, used at `:129-130`).
- **Revision.** The catalog-wide revision of Part 1. It counts every committed transaction that changed something, from any
  process; each read returns the revision it read at.
- **The watcher** (`pg-catalog`). One thread in the `Database`, with its own read connection, started only when the
  `Database` is given a function to report to. With the writer, it notices another process's writes (R3a).
- **Qt-side values, in two layers.**
  - *Stored.* Each value the GUI holds from the catalog stays where it lives today, and gains the revision it was read
    at: the grid's listing and the listing held under the viewer (`pg_client::Grid`, `crates/client/src/grid.rs:109`,
    `:121`), the details slots, the folder tree, the facets, the saved views (`Shell`), and the roots (`Library`).
  - *Pending.* One set on the Qt thread holds every edit made and not yet acked or failed, whatever it names: a file, a
    view, the session, a folder. That includes an add whose command is not yet sent (R7). The grid is given the file commands to draw over its entries. The cap reads this one set (R5). The close's
    decision and the screenshot harness read it together with the count of model commands posted and not yet run
    ([Shutdown](#shutdown)).
  - What shows is the stored value with the pending commands applied on top, with two exceptions that R4 gives: the
    image key and aspect, and the roots list, show the stored value only. Reads and acks change only the stored layer;
    commands and acks change only the pending set.
  - *They are plain types.* The pending set, the 200 ms mark (on virtual time, like `Deadlines`) and the close's
    decision live in `pg_client`, or as free structs in the desktop crate, not inside a QObject. The desktop's tests
    construct no QObject today (`shell.rs:572-619` drives a bare `Controller` and free functions), so that is the only
    place their tests can live.
- **Providers.** The image providers do not use the point lane. They read through `Images` and the catalog pool on their own
  threads, as today (`crates/app/src/images.rs`). The pool's connections become read connections.
- **Probe threads and Part 5's pool** (`pg_app`). A file or volume probe that can hang (a dead network drive) runs on a
  thread of its own, started for it, so it is never queued behind another probe and holds nothing else back (R7). A
  probe that never returns is abandoned at exit. The other Part 5 requests run on a small pool.

## Rules

- **R1, a read never waits and never hides a pending edit.** A read result always goes to the stored layer, under R2.
  The pending set is drawn over it, so a file with a pending command keeps showing its pending value (R4). No result is
  dropped or merged because a command is pending.
- **R2, every stored value carries its revision and only moves forward; a change that a result missed is read again.**
  - *Every stored value.* The grid's listing, the held listing, the details slots, the tree, the facets, the saved
    views and the roots each carry the revision they were read at. A result or an ack applies only if its revision is at
    least the held one. A refused result is always older than what is held. So a roots read made before a folder's
    removal cannot land over the removal's ack.
  - *Entries replaced on their own.* An entry replaced since its listing was read (by an ack or a point read) carries its
    own revision in a sparse map beside the listing. When a new listing is applied, an entry whose own revision is above
    the listing's keeps its value at the row's new position, and stays in the map; the entries at or below the listing's
    revision leave it. The map is never simply emptied: an older listing would then let an older point read land over a
    newer ack.
  - *A change to a file is read.* `Turned`, or an ack, names a file. If its revision is above the held one for that
    file's entry or for a details slot holding that file, the GUI asks the point lane for what the event did not itself
    carry: after `Turned`, the entry and the details; after an ack, which carries the entry, the details only. Nothing
    else is read: the facets, the tree and the listing are not re-read for one file's change (R4). One at or below the
    held revision asks for nothing.
  - *Changes noted while a read is in flight.* A read is *in flight* for a kind of value from when its newest request is
    sent until that request's result arrives, whether it is then applied, refused or an error (a listing held under the
    viewer excepted, below); a request that was
    replaced or cancelled is not in flight, its replacement is. (The tree and the facets already count generations,
    `folders.rs:174-199`, `filters.rs:154-183`, which serve for this.) While any read is in flight, the GUI notes each
    `Turned` and each ack: one note per file, with the newest revision, so the notes are bounded by the files that
    changed. When a result is applied, every file in it whose noted revision is
    above the result's is asked for again through the point lane. The notes are dropped when nothing is in flight.
  - *A change to everything is not a note.* `Foreign` and a lag are the only causes that name no file. They are kept as
    causes of the spaced refresh (R3), not noted: a listing that lands while one is kept is not asked for again at once.
  - *A listing held under the viewer* is still in flight. It arrives while the viewer is open, is held
    (`crates/client/src/grid.rs:285-288`) and is applied at release (`gallery/model.rs:122-126`). Its flight ends when it is
    applied or replaced, so the notes made meanwhile are still there for it.
  - *An applied listing asks for the details again.* Today applying a listing forgets the details
    (`forget_details`, `gallery/model.rs:146`), so every reload re-reads them. Here each details slot whose revision is
    below an applied listing's is asked for again. Otherwise the info line would mix the new entry with old details
    after an import (`gallery/items.rs:184-193`).
  - *An error result* ends its read's flight, changes no revision, and leaves the sparse map alone. A failed listing
    stores the empty list at the held revision, so it shows as empty, as today (`gallery/model.rs:104-108`); any other
    failed read changes no stored value.
  - The notes generalize what the grid does today for turns (`note_turns_written`, `take_turned` and `refresh_turned`,
    `crates/client/src/grid.rs:345-354`, `desktop/src/gallery/model.rs:158-160`), and they cover what a revision check
    alone loses: a file changes while a listing that contains it is loading, and the older snapshot then lands.
  - A result's revision is the one its own read transaction saw (Part 1), so it bounds the data the result holds.
- **R3, lost events.** The bus drops the oldest event for a lagging subscriber and counts the drops
  (`crates/jobs/src/events.rs:82-85`). The library pump reads that count (Part 3). When it is above zero, the GUI refreshes
  everything (below). The results apply under R2.
  The inbox's `Changed` events carry the whole listing, and its pump keeps only the newest (`desktop/src/notices.rs:261-265`),
  so a lost one costs nothing. A lost `Toast` is dropped silently (the inbox bus holds 256 events,
  `crates/app/src/inbox.rs:303`); the pump logs the dropped count. The window shows only the last toast of a pass anyway
  (`desktop/src/notices.rs:294`).
  - *"Refresh everything"* is one list, used by R3 and R3a: the grid's listing and the folder tree (through the listing
    lane, as `Scanned` does today, `desktop/qml/Main.qml:127-128`, `:255-265`), the facets (`Main.qml:142-145`), the details
    slots, the views, the roots, the status lines, and the inbox's poke (`desktop/src/library.rs:134-160`). It re-lists,
    so it drops a file rated out of the filter, as any reload does today.
  - *It is spaced.* A full listing of the large root takes up to 430 ms (`crates/app/src/reader.rs:178-183`) and a full
    layout pass on the GUI thread. A refresh is not started while the last one is in flight, nor within 5 s of the last
    one's start; a cause that arrives meanwhile is kept, and one refresh follows. The 5 s is chosen, not measured.
- **R3a, writes from another process.** The command-line tool writes the same catalog through its own `Database` and its
  own writer: scan, import, remove, the pending marks, borders, thumbs and the write-back (`crates/cli/src/lib.rs:218`,
  `:228`, `:248`, `:279`, `:324`). The screenshot fixtures (`tools/shots/fixture.py:82-102`) write rows directly with no
  revision; they run with no window open.
  - *`data_version` alone is not the test.* It moves whenever any other connection commits
    (`crates/app/src/inbox.rs:716`), and that includes this process's writer.
  - *The test is a revision this process did not write.* The `Database` keeps the last revision it has seen, raised only
    upward. It starts as the stored revision, read in the writer's first transaction, so nothing is reported at open.
    Two places compare the stored revision with it:
    - the writer, at the start of every transaction. If the stored revision is not the last seen, another process
      committed in between. The writer raises the last seen to it at once, whether or not its own transaction goes on to
      bump, commit or roll back, and reports it when the transaction ends. This catches a foreign write that an own
      commit follows before the watcher looks. It is built in Part 1, where it reports to nobody: the last seen revision
      is kept from the start, so Part 3 can add the route without a false first report. The watcher's thread and the
      commit counter below are built in Part 1 too, with the `Database`; the app gives it a function only from Part 3;
    - the watcher, which reads `data_version` once a second (it costs microseconds) and, when it has moved, reads the
      stored revision. If that is above the last seen, it raises the last seen to it and reports it. This catches a foreign
      write while the writer is idle.

    Each of these two raises is an atomic maximum, and of the two only the side whose raise changed the value reports.
    So one foreign revision is reported once, whichever sees it first. The writer's raise after its own commit (below)
    never reports: that revision is this process's own.
  - *The watcher must not see an own commit as foreign.* The writer raises the last seen revision after its commit, so
    the watcher could read the new stored revision in between. The writer therefore counts its commits in a sequence
    counter, raised before `COMMIT` and again after the last seen revision is raised, also when the commit fails. The
    watcher reads the counter, then the stored revision, then the last seen revision, then the counter again, and tries
    again when the two counts differ or a commit is under way.
  - *The route.* The `Database` calls a function the app gives it. The app publishes `Change::Foreign { revision }` on the
    change bus, and the library pump answers it by refreshing everything, spaced as R3 says. A command-line pass commits every second or
    two (an import's flushes, a thumbs pass's ledger, a scan's batches), so without the spacing it would refresh the
    window about once a second for its whole run. The app creates the bus in `App::open`; today `Library::start` creates it
    (`crates/app/src/library.rs:99`).
  - *The inbox is not changed by this.* Its own `data_version` comparison (`inbox.rs:803-804`) also tells it of this
    process's ledger writes (`inbox.rs:8`), and it keeps it.
  - Another process's write therefore reaches the window within about a second when no refresh started in the last 5 s,
    and within 5 s otherwise. The same spacing applies to a lag: a `Turned` the bus dropped is made good by a refresh,
    so those files can show their old picture for up to 5 s, where today's re-read is immediate.
- **R4, pending is shown and marked, and the stored layer changes only at the ack.**
  - *A pending value is marked.* It is dimmed, and assistive technology is told it is pending. The mark appears only
    after the command has been pending for 200 ms: an ack normally lands within a loop turn or two, and a mark shown at
    once would flicker on every edit. So for up to 200 ms a new rating shows unmarked. The 200 ms is chosen, not
    measured. One timer in the model serves every pending command; no row has its own.
  - *The ack* carries what the catalog now holds, read inside the command's own transaction, so it is one snapshot at the
    ack's revision. It goes to the stored layer under R2, and the command leaves the pending set. A command that changed
    nothing is acked with the revision it read. What each ack carries:
    - a file command: the file's entry. An ack for a file that is in no listing the GUI holds has nowhere to apply; the
      command just leaves the pending set;
    - a view command: the saved views, whole. `views().all()` already returns them (`crates/app/src/views.rs:46-63`), and
      with them held, applying a view stays an in-memory lookup in the shown views (a pending save carries its view, so
      a view applied before its save is acked is found): `shell.applyView` is called synchronously from QML
      (`Main.qml:275-285`, `shell.rs:375-381`). The session's save: nothing;
    - adding a folder: the roots, the root's normalized path and whether it was added, as `add_folder` returns today
      (`crates/app/src/library.rs:298`, used at `desktop/src/library.rs:212-223`). Of today's three refusals, the two that
      come from normalize and `check_folder` (`crates/library/src/folders.rs:30-32`) are found by the probe and never
      reach an ack (R7); a catalog error is the command's failure;
    - removing a folder: the roots and the removed root's path. On that ack the `Library` object emits `scanned()`, as it
      does today on success (`desktop/src/library.rs:231-234`), which reloads the tree and the grid.
  - *A pending rating or border mark* shows its new value at once.
  - *Whatever reads an edited field reads the shown value*, not the stored entry. The border toggle reads the mark to
    decide what to send (`shell.rs:486-488`, `gallery/edits.rs:98`); from the stored entry, a second toggle while the
    first is pending would send "mark" again. The crop's ids (`edits.rs:120-121`), the viewed item's rating and mark, and
    the selection line's symbol (`gallery/model.rs:266-272`, `:297`, `:324`) read it the same way, and so does the info
    line (`gallery/items.rs:185-191`). One accessor gives the shown rating and mark and the stored
    value for everything else, and every reader above goes through it. So do the row's roles, which most need the pending
    value: `rating()` and `border_marked()` (`gallery/items.rs:109`, `:130`, reached from `gallery/model.rs:54`) become
    that accessor, which covers the rows, the viewer and the selection line. The layout's `plan` reads only the aspect
    and stays on the stored entry, at no cost. Today they do not share one: the toggle reads
    `self.grid.item(at)`, the crop reads `self.grid.entries()`, and `entry(item)` (`gallery/state.rs:97`) returns the
    stored `&Entry`. The image key and the aspect keep reading the stored entry.
  - *The roots list stays the stored one.* The library panel lists `library.roots` (`LibraryPanel.qml:46-57`), and a
    removal maps its row to `root_ids[row]` (`desktop/src/library.rs:96`, `:227`). If pending changes altered that list,
    a removal could hit the wrong folder. So `roots` and `root_ids` are the stored list only. A pending removal dims its
    row and disables its button. A pending add shows as a separate line below the list, which no row index reaches.
    - "Rescan all" is disabled while a removal is pending. Today both calls are synchronous; with the removal a command,
      the rescan's roots read could come before the removal's commit and its scan after `folder_removed`, and
      `begin_scan` does not check that the folder is still a root (`crates/catalog/src/roots.rs:79-100`).
    - With the first add pending, the empty states say so: the grid's and the panel's "no folders" texts
      (`Main.qml:562`, `GalleryGrid.qml:97`, `LibraryPanel.qml:60-64`) show the pending add in their place.
    - A folder that was already a root is reported as today (`FolderAlreadyAdded`, `desktop/src/library.rs:215-217`): the
      ack carries the flag, and the ack's handler posts the notice.
  - *A pending turn does not turn the picture.* The image key is `<id>@<mtime>.<rotation>.<pending_turns>`
    (`desktop/src/gallery/items.rs:41-46`), and on a thumbnail miss, the placeholder and the full image the providers turn
    by what the catalog holds (`crates/app/src/images.rs:227`, `:272`, `:290`). A key that ran ahead of the catalog would
    make Qt cache unturned pixels under the turned key. So the key and the layout's aspect (`Entry::aspect`,
    `crates/model/src/grid.rs:75`) come from the stored layer only. A pending turn shows as the pending mark alone.
  - *A turn has two later moments.* At the ack, the stored entry holds the queued turn (`rotation` is `files.rotation`
    and `pending_turns` is the queued `file_writes` value, `crates/catalog/src/browse/read/grid.rs:26-27`, `:68-69`), the
    key changes, and every view turns, as today after `queue_rotation` commits (`desktop/src/gallery/edits.rs:27-47`).
    At `Turned`, the write-back has rewritten the file, and the entry is read again (R2).
  - *A command that fails* leaves the pending set, and the stored value shows again. No read is needed: the stored layer
    never held the failed value. The user is told as today, by the edit's failure notice (`edit_failed`,
    `desktop/src/gallery/edits.rs:27-30`).
  - *A rating does not re-list.* A file rated out of the current filter stays until the next reload, and the facets stay
    as they were (`crates/client/src/grid.rs:393-395`, `Main.qml:260-262`). That is today's behaviour, kept.
- **R5, the lock is busy only when another process holds it.** Inside the process the writer is alone, so a busy write
  means the command-line tool holds the lock.
  - *The wait.* The writer's connection has a busy handler in place of the 5 s timeout (rusqlite's `busy_handler` takes a
    closure, `src/busy.rs:55-57`). It sleeps and tries again, and gives up when the current call's stop is raised. It is
    installed after `SETTINGS`, because `PRAGMA busy_timeout` replaces it (`crates/catalog/src/schema.rs:377-378`; so does
    `Catalog::set_busy_timeout`, `core.rs:190`). During `Database::open` the handler's slot holds the open's own stop,
    from the handler's installation until the open returns: the migration step begins a write transaction on every open,
    even with nothing to migrate (`schema.rs:410-414`), before the writer takes any call. So that wait, too, ends on the
    open's stop and not after 5 s. No write is lost to the busy timeout; a write ends unwritten only when
    its own stop is raised.
  - *The busy state.* The writer tells a function when it has waited 10 s on another process, for a write of any class,
    and again when that wait ends: the lock is had, or no write is left waiting for it. The desktop passes that
    function into `app::open`, which passes it to `Database::open`: the placeholder needs it before the app exists. The
    function runs on the thread that is waiting (the writer's, or the opening thread for the file lock and the migration step) and posts to the
    Qt thread, to whichever object is current: `Startup` before
    Ready, the `Shell` after. One process-wide holder, under the same lock as the start-state slot, names that object
    and keeps the current busy value, so Ready and the busy state cannot race `Startup`'s construction, and an object
    that registers later (the `Shell` at Ready, or after a live reload) starts with the right value. The 10 s counts the
    writer's unbroken wait on another process, not one write's: when a write is stopped and the next takes over the same
    wait, the state does not clear and start again. The window shows a busy state: a QML property
    set on the Qt thread, not a database notice, since a notice is a write the same lock would hold. A transient toast
    says so. Before Ready the placeholder shows it (R9). The 10 s is chosen.
  - *The cap is the GUI's.* The pending set holds at most 10,000 file commands. A file command beyond that is refused on
    the Qt thread, before it is sent. Commands that name no file are few (about one session save a second at most) and
    are not counted, so the cap never refuses the close's own session save. The value is chosen, not measured.
  - A command that fails for another reason (a constraint error) fails at once. Shutdown is in [Shutdown](#shutdown).
- **R6, coalescing per file and operation.** The command thread coalesces the commands still waiting in its own queue, per
  (file, operation) pair, not per file alone. Commands that name no file (a saved view, the session, a folder) coalesce
  per operation and key: the session keeps the last value, and a view keeps the last command for its name.
  - turns add up, mod 4, as `queue_rotation` does (`crates/catalog/src/queue.rs:103-125`); a sum of zero is still sent, so
    its commands are acked;
  - a rating or a border mark keeps the last value.
  Every command gets its own ack message, by its id; the commands one write carried get the same snapshot. If that write
  fails, they all fail.
- **R7, a multi-step operation is one command, its probes run first, and its write is one transaction.**
  - *Adding a folder.* It is pending from the click. Normalize and the NTFS checks (`check_folder`,
    `crates/scan/src/lib.rs:281-283`, which calls `refusal::check_root`) run first, on a probe thread. The thread posts its result to the
    Qt thread. If the add is still pending and the close's decision is idle, the Qt thread sends the command.
    Otherwise the result is kept and nothing is sent ([Shutdown](#shutdown)). The command's whole work is one short write transaction (`add_root`,
    `crates/catalog/src/roots.rs:39-53`, which also deletes the roots nested in the new one). After the commit the command
    thread queues the scan (`Runner::scan`). A refusal by the probes is that pending item's failure. A probe of a dead
    drive may not return for a long time, and the add stays pending meanwhile. Nothing has been written for it, so an
    unsent add does not hold a close, and is dropped if the window closes ([Shutdown](#shutdown)).
  - *Removing a folder.* One command, one transaction (`remove_root`, `crates/catalog/src/roots.rs:58-72`), which deletes the
    root row, sets `missing = 1` over the folder's files, and cascades the root's notices and folder-ledger rows by foreign
    key. Today `remove_folder` reads the roots in a separate statement first (`crates/app/src/library.rs:315-322`) and
    passes the root's path to `Runner::folder_removed`, which stops a scan under it. The command drops the outer read:
    `remove_root` reads the path inside its transaction and returns it, and the command thread calls `folder_removed` with
    it after the commit. The measured cost is 2.3 to 3.6 s for the 189,519-file root and 0.9 to 1.0 s for the 41,293-file
    root (debug, one machine; not measured in release). Every write queued during it waits for it; none fails. A rating
    made then stays pending for that long. Whether one writer makes it safe to split the removal into shorter
    transactions is not designed ([Open](#open)).
  - *Rescanning* is not a catalog write. It reads the roots at a revision (a point-lane request) and queues the scans in the
    library job, through a Part 5 request.
- **R8, blocking calls are checked.** The blocking entry points move into a `providers` module of `pg_app`'s facade, which
  the GUI modules do not call. The desktop's providers call them (`crates/desktop/src/providers.rs:44-119`). The facade
  checks the calling thread's role on every blocking call, in every build, and `write` checks it in `pg-catalog`: it
  refuses the GUI thread and the writer's own thread. The image providers' Qt pool threads (`Job`,
  `crates/desktop/cpp/providers.cpp:98-120`) run `Job::run()` and call into Rust, so they are workers and may block. A
  blocking call from the GUI thread logs an error and returns the failure the caller already handles. It does not panic: a
  panic across the cxx-qt bridge aborts the process. One facade entry is exempt: the session-end wait on the command
  thread and the preferences writer, which runs on the GUI thread by design ([Shutdown](#shutdown)).
- **R9, startup.** `app::open` runs on a startup worker, which then reads the start state. The window shows a loading
  placeholder until both are done.
  - *The start state is read on the worker, not queried by the GUI.* `ShellRust::default` returns the listing, the panels
    and the size synchronously (`crates/desktop/src/shell.rs:39-56`), from `Core::new`, which needs the session's listing,
    the applied view and the preferences before it builds the controller (`crates/desktop/src/controller.rs:38-66`).
    `Filters`, `FacetChoices` and `FolderModel` reach the same core (`filters.rs:32`, `facet_choices.rs:54`,
    `folders.rs:236`). There is no "loading" controller, and the plan adds none. The worker reads the session's listing,
    the applied view, the saved views and the preferences, leaves them in a slot beside the app, posts Ready and ends.
    `Core::new` and `view_names` read the slot; they do not empty it. A failed read gives the default state and a notice,
    as now.
  - *The app stays a process-wide value.* The bridge objects are QML elements that QML constructs through `Default`
    (`#[qml_element]` in every `*_bridge.rs`), so they cannot be handed the app when they are made. `app::get()` stays, as
    `APP.get().expect("app::open not called")` (`crates/desktop/src/app.rs:44-51`).
  - *So no bridge object that reaches the app may exist before Ready.* Today `app::open` runs on the main thread before
    the GUI application exists (`crates/desktop/src/lib.rs:164`). During QML load the app is reached by `Shell` (in
    `Default`), `Settings` (on first property read, `settings.rs:22`, `:26`), `Notices` (`start`, `notices.rs:254`),
    `FolderModel` (`reload`, `folders.rs:180`), and `Filters` and the six `FacetChoices` (through the core). `Library`
    reaches it later, from `startHooks`.
  - *One engine load, with a `Loader`.* The root window creates one bridge object, `Startup`, which reaches nothing. A
    `Loader` creates the main content when `Startup` reports Ready. The app and the slot are set before
    Ready is posted, so the `expect` cannot fail by construction. A `Startup` constructed after that, by a live reload
    (`cpp/live_reload.cpp:100`), finds the slot set and reports Ready at once. The controller's core is built once per
    process (`CORE`, `crates/desktop/src/controller.rs:30-33`), so a reload reads only the saved views from the slot
    again (`shell.rs:48`); they are kept current there by each view command's ack.
  - *All of `Main.qml`'s content moves into the loaded component.* What the root must keep or forward: the title (bound
    to `shell.folder`, `Main.qml:21`); `current` (an alias into the model, `:47`, which cannot reach into a `Loader`);
    `ready` and `shellCore`, which `cpp/screenshot.cpp:82`, `:177` read (`:74`, `:116`); `onClosing`, `onActiveChanged`
    and `onDevicePixelRatioChanged` (`:138`, `:369`, `:734`); and `Component.onCompleted` (`:736-748`). A `Loader` parents
    its item, so the harness's `findChildren` still finds the objects. The trace stages change meaning: `firstFrame`
    (`:117-124`) moves to the loaded content, or it would time the placeholder, and `init_done`, `qml_load` and
    `qml_loaded` (`lib.rs:167`, `:188-190`) are renamed for what they now time.
  - *Waiting behind another process.* `Database::open`'s writes carry the worker's stop. If the command-line tool holds
    the lock, the placeholder shows the busy state of R5 after 10 s, through the function the desktop passed in.
  - *The file locks are a second wait.* Before any SQLite lock, `prepare` takes the preparer's lock
    (`crates/catalog/src/core.rs:69`, `lifecycle.rs:43-47`) and at the end the shared hold (`core.rs:124`). The
    file-lock layer waits in its own loop of 100 tries 50 ms apart (`crates/dbfile/src/lock.rs:12-13`, `:29-37`,
    `:57-65`), which SQLite's busy handler never sees. Only the preparer's lock can wait there: the shared hold is taken
    while that lock is held, and the only exclusive taker is `prepare` itself (`core.rs:92`), which does not wait.
    Behind another process's preparation the open would still fail after about 5 s with the fatal box. So the catalog's
    `Lock::take` takes the open's stop too, and reports to the same busy
    function: the wait then ends on the stop, and shows the busy state, like the SQLite one. A preparer holds its lock
    for as long as its own marks wait, which for the command-line tool has no limit; the busy state is what tells the
    user.
  - *A failed open.* Today it is fatal before Qt exists: a native error box, then `process::exit(1)`
    (`crates/desktop/src/lib.rs:164-166`, `alert.rs`). There is no app and so no inbox for a notice. The worker reports
    the failure in place of Ready; the GUI thread shows the same box and then leaves through `QCoreApplication::exit(1)`,
    since Qt and its render thread are running by then. A panic on the worker is caught there and reported as a failed
    open: uncaught, it would end only the worker, and the placeholder would stay up for good.
  - *A stopped open is not a failed open.* When the open returns `Stopped`, the user asked to close before Ready
    (below). There is no box: the ordinary shutdown runs.
  - *Read connections opened later do not wait.* The `Database` keeps its shared hold from the end of the open, so no
    other process can take the exclusive hold while it lives. A read connection opened by the pool on a provider thread
    therefore cannot sit in the file-lock loop. The thumbnail cache takes the same kinds of lock on its own file
    (`crates/thumbcache/src/schema.rs:245`, `:371`); it keeps today's bounded form, since a cache that fails to open is
    only disabled.
  - *A close before Ready.* There is no `Shell` and no controller yet, so the close's decision of
    [Shutdown](#shutdown) does not apply. `Startup` holds a closing state of its own: the root's `onClosing` asks it, with
    no flush and no controller event. The window stays, saying it is closing, and the worker's stop is raised. A write
    or a file-lock wait that is waiting ends at once; a migration or a transaction in progress runs to its end, and
    cannot be cut short, so the open may still succeed. Whatever the worker then reports (Ready, a failed open, or a
    stopped one), nothing is loaded and no box is shown: the
    ordinary shutdown runs. `app::stop` and `app::shutdown` already tolerate a missing app
    (`crates/desktop/src/app.rs:55`, `:64`). The window does not vanish while the process still holds the file.
  - Once loaded, the first content frame shows the start fields as today. So R9 is a cost the goal forces, not a fix for a
    visible flash.
- **R10, visibility.** The public surface of `pg_app` is its message types and its facade (the blocking `providers` module
  is part of the facade). The catalog data types the front
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

Parts are in dependency order: no part uses what a later one builds, and the app works after each one.

- Part 1 comes first because everything else stands on it: the revision is bumped in one place only once there is one
  writer. It includes the command thread, because Part 1 makes the pool read-only and the GUI's writes need somewhere to
  go, and the thread role and the panic marker, because `write` checks the one and the writer sets the other.
- Part 3 builds the read side with its revisions and R2, so that Part 4 can stop waiting: a write whose result the GUI
  cannot read back consistently is worse than a write that waits.

### Part 1, the `Database`: one writer, read-only connections, the revision

**The `Database`, the writer, the thread role, the panic marker and the command thread** are as in [Design](#design).
Every site that opens a connection or begins a write transaction today moves onto them:

| Today | Site | Becomes |
| --- | --- | --- |
| the job's connection, kept between rounds | `round`, `crates/library/src/runner.rs` | a read connection; each pass's transactions are `write(job, ...)` |
| the scan | `lease.tx()`, `crates/scan/src/lib.rs:623`; `begin_scan`, `finish_scan`, `mark_missing` and `folder_refused`, `crates/catalog/src/roots.rs:79-163`; the pairing tiers, `crates/catalog/src/pair.rs:266-283` | job writes; the lease is below. `pair_after_scan`, `in_batches` (`pair.rs:79`, `:266-283`) and `mark_missing` (`roots.rs:118-155`) mix reads with several write transactions, so they take a read connection, the writer's handle, the class and the stop |
| the import | `catalog.tx()`, `crates/import/src/lib.rs:51` | one job write per flush (Part 2a) |
| the borders pass | `catalog.tx()`, `crates/borders/src/lib.rs:61` | job writes |
| the thumbnail pass | its own `Catalog::open`, `crates/thumbs/src/lib.rs:202` | a read connection; its only catalog writes are the failure ledger's (`record_failures`, `clear_failures`), which are job writes. The cache's attach stays as it is ([Design](#design)) |
| the write-back | its own connection, `crates/writeback/src/lib.rs:326`; `fault.rs:73-74` | a read connection; `write_done` and the queue's writes are service writes |
| the inbox | its own connection, `crates/app/src/inbox.rs:742` | a read connection; each step stays its own transaction, as today (`inbox.rs:813-849`), and is one service write |
| the listing lane | `crates/app/src/reader.rs:172` | a read connection |
| the GUI's pool writes | `crates/app/src/edit.rs:29`, `:36`; `views.rs:42`, `:52`, `:57`; `library.rs:300`, `:315`, `:338` | commands (below) |
| the pool | `Catalogs::with_catalog`, `crates/app/src/catalogs.rs:38-43` | read connections only |
| the keeper | `pg_library::open_catalog`, `crates/app/src/app.rs:62` | `Database::open`; the writer's connection takes its place |
| the command-line tool | `pg_library::open_catalog`, `crates/cli/src/lib.rs:136` | `Database::open` |

Crops are not in the table: `pg_crop::save` only reads the catalog (`crates/crop/src/lib.rs:68-78`). Outside `pg-catalog`
the calls to `Catalog::open`, `prepare` or `open_catalog` number about 30 in `library`, 8 in `app`, 4 each in `cli`, `scan`,
`thumbs` and `bench`, 2 in `writeback` and 1 in `thumbcache`; `pg-catalog` itself has about 90, most in tests.

**The GUI's writes, until Part 4.** The GUI's calls keep their signatures. Each sends its command to the command thread
with a 5 s deadline as its stop, and waits on the GUI thread for the reply, which comes over a channel (the Qt thread is
blocked, so nothing can be posted to it).

- The reply is definite: the ack, or `Stopped`. A command that was still waiting at 5 s is withdrawn and fails, as a busy
  write fails today. A command whose transaction had begun is waited out, so the GUI never reports a failure for a write
  that then commits.
- The command thread already does what follows a commit (the write-back's wake, the scan, `folder_removed`), on its own
  thread. The GUI's own code after the call is unchanged until Part 3, where the ack replaces it: it refreshes as it
  does today.
- An add's probes stay where they are until Part 4: on the GUI thread, before the command is sent. The command is only
  the `add_root` write.
- Coalescing (R6) is in the command thread from the start but cannot happen yet: the GUI sends one command and waits.
- So the GUI behaves as today after Part 1. The job does not: its writes now wait on another process instead of failing
  after 5 s (Part 2a). Part 4 removes the GUI's wait, and Part 6's check then forbids it.

**Opening, until Part 3.** `app::open` still runs on the main thread, before Qt (`crates/desktop/src/lib.rs:164`). Two
things follow:

- its `Database::open` is given a 5 s deadline as its stop. With the busy handler in place of the timeout (R5), a lock
  held by the command-line tool would otherwise hang the process with no window; with the deadline the open fails as
  today, with the fatal box;
- the main thread takes the GUI role only after `app::open` returns, or `write` would refuse the preparation's writes.

**The preparation's writes go through the writer.** `Catalog::prepare` writes today, each in its own immediate
transaction: `mark_unheld_missing` and the import, borders, thumbs and taken-zone marks (`crates/catalog/src/core.rs:115-119`).
They run even when another process has the file open; only a rebuild or a migration needs the exclusive hold
(`core.rs:91-95`). In `Database::open` they are the writer's first transactions: `write` calls made by the opening thread,
of the job class, with the stop `open` was given ([Design](#design) gives the order). They bump the revision, so once Part 3 is built a running window is told (R3a). `set_import_version` rewrites
every file row twice (`crates/catalog/src/lifecycle.rs:76-84`), so a second process starting can hold the lock for seconds.

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

**The write-back and `Stopped`.** A `write_done` that is stopped after the file was rewritten does not leave the catalog
and the file apart: the temp is recorded (`crates/writeback/src/lib.rs:579`), and `finish_copy` (`:649`) completes it at
the next start. But the error branch (`:416-445`) calls `write_failed`, and `run` sets the status error (`:307-311`). `Stopped`
is neither an attempt nor an error: the write-back ends its work there.

**The job and `Stopped`.** In the job any `Err` from a pass becomes a notice: `RoundFailed`, `ScanFailed`,
`LedgerNotWritten` (`crates/library/src/runner.rs`, by symbol). `Stopped` is not a failure. It ends the pass as a
preemption does (`Ok(Step::Preempted)`), with no notice. `pg-catalog`'s error gains the variant, with a test like
`Error::is_busy`; the passes use `anyhow`. Three places flatten or misroute an error today and must test for it first:

- the scan's `ScanError::Failed`, which posts `ScanFailed` and calls `scanned()`;
- the thumbnail pass, which turns the ledger's error into a `String` inside `pg_thumbs` (`crates/thumbs/src/lib.rs:77-78`),
  so `LedgerOutcome` gains a stopped case, which the command-line tool also reads (`crates/cli/src/lib.rs:267`);
- the write-back's error arm, above, which itself writes.

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
  under the exclusive hold before the writer exists; the migration step's transaction, which the opening thread runs
  on the prepared connection before it moves it to the writer; and the `PRAGMA` statements.

**Tests and fixtures.**

- *Tests use files.* About 30 sites open a `:memory:` catalog (27 in `catalog`, 2 in `scan`, 1 in `bench`), among them the
  shared fixture (`crates/catalog/src/test_support.rs:33`). A `:memory:` database has one connection, and the design needs
  a writer's and readers'. Those tests move to catalog files in a temporary folder (`pg_testkit::run_folder`; `pg-testkit`
  is already a dev-dependency of `pg-catalog`). The product code gains no single-connection mode, and loses the one it
  has: the `memory` branches of `core.rs` (`:68-69`, `:92-93`, `:104`, `:124`, `:136`) and the test of that mode
  (`lifecycle.rs:955-957`) go. About 250 catalog tests each gain a folder, a file and a writer thread; the cost is not
  measured ([Open](#open)).
- *Fixtures write through `write`.* They are gated by `feature = "fixture"`, not `cfg(test)`
  (`crates/catalog/src/lib.rs:21`, `transaction.rs:42`), and the app's tests call them on pooled connections
  (`crates/app/src/images.rs:344-349`, `:401`, `:434`, `:469`; `app.rs:238`).
- *The hold timing moves to the `Database`.* It is a thread-local today (`WATCHED`, `transaction.rs:47-50`; the
  200,000-file scan test asserts on it, `crates/scan/src/lib.rs:1318-1324`), and the holds are now on the writer's thread.
- A test's `Database` starts a writer thread, and a watcher only when the test gives it a function to report to, as
  R3a's tests do. Two `Database` values on one file in one process work: the second finds the exclusive hold refused,
  and its preparation changes no row when the marks match, so it bumps nothing. The benchmarks' baselines will include the hand-off to the
  writer.

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
transaction, `crates/catalog/src/schema.rs:401-435`). `meta` is a key and value table with no triggers on it. The existing
`meta.seq` (`crates/catalog/src/sql.rs:17-19`) is not it: that is the thumbnail clock, bumped by triggers and `set_mark`
and read by the thumbnail cache. An older build refuses a newer schema (`crates/catalog/src/core.rs:75-81`, `:143-148`), so
no process writes the file without bumping, apart from the screenshot fixtures (R3a).

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
- **The write.** One job write. Store the analyses (`store_analysis`, `crates/catalog/src/writers.rs`, with its
  `IS_CURRENT` test). Store a pair only when both of its files were stored. A rejected file's pairs are then dropped, and no
  other pair changes, because a pair is a relation between two files. `store_analysis` deletes every pair naming its file
  (and so does `store_failure`), so the batch's pairs are written after both files' analyses, in the same
  transaction. The write returns which files were stored.
- **After commit.** Insert the stored candidates' orientations (`Pairer::insert`). The room is checked before the
  write: `u32::try_from` of the index's length plus the orientations must succeed, the u32 link limit. (`reserve()`
  only grows capacity, `pdq_index.rs:199`.) So this insert cannot fail with `IndexFull` (`pdq_index.rs:213`, `:274`). The
  index holds only files that are not pending import (`crates/catalog/src/work.rs:124-126`), so a file being imported is
  never in it already: nothing needs removing and nothing is inserted twice.
- **Equivalence.** Sequential query-then-insert pairs each new file with the earlier stored files. Batch matching gives the
  same pairs for two reasons, and neither is the order test:
  - the index is exact, not approximate (multi-index hashing with `MAX_RADIUS = 31 = MATCH_DISTANCE`,
    `pdq_index.rs:1`, `crates/model/src/pdq.rs:7`), so a brute-force comparison inside the batch finds what a query of the
    index would. `a_built_index_finds_exactly_the_brute_force_set` (`pdq_index.rs:452`) tests that;
  - the later file is the query in both, so the stored orientation is the same. The variant is the query file's
    orientation, ties keep the lowest (`d < b.1`, `pdq_index.rs:295`), and `store_pdq_pair` orders the ids but not the
    variant (`crates/catalog/src/writers.rs`).

  A rejected file's pairs are not stored in either: today a rejected file skips pairing (`import/src/lib.rs:57-60`).
  `pairs_do_not_depend_on_import_order` (`pdq_index.rs:386-414`) does not cover this. It compares only the set of id pairs
  across sequential orders, on `PdqIndex`, without distances or variants.
- **A neighbour that changed.** "Both files stored" covers the batch's own files. A pair with a file already in the index
  is stored even if that file changed between the matching and the commit. Whether today's import has the same window,
  and whether the write should re-check the neighbour, is not traced ([Open](#open)).
- **Failure.** A commit that fails leaves the index unchanged, because nothing was inserted. The run ends at that batch, as
  today; today the index is simply dropped with the run.
- **Busy.** The import has no busy policy of its own. Its write waits in the writer's queue, and the writer waits on
  another process until the job's quit and its grace have passed (R5). Today a busy batch fails after the 5 s timeout and ends the run
  (`pipeline.rs:262`).
- **Cost.** Batch matching is in memory: about 125,000 pairs of 64 orientation comparisons per 500 files. Not measured. The
  writer's hold is the SQL alone: flush minus pairing was 0.19 to 0.56 s per batch (second pass, release).
- **Tests.** A failed commit leaves the index unchanged. A pair naming a rejected file is not stored. A new equivalence
  test: the same files, imported one per batch and all in one batch, store the same pairs with the same distance and
  variant, including two files of one batch that match each other. The order test still holds.

**2b, the checkpoint yields to edits.**

**Precondition.** In each measured run one turn stalled 100 ms or more, at the first 1000-frame checkpoint. Every turn
was timed and no other stalled, so the about 20 checkpoints that should have followed in the same run cost under 100 ms
each, if they ran ([Measurements](#measurements)). What is not known is whether they ran, what each cost, and what the
first one paid for. Measure that first ([Open](#open)). If only the first one costs, SQLite's automatic checkpoint stays
as it is and this part is not built.

If they do stall, an automatic checkpoint runs on the writer's thread after a commit and delays whatever is queued behind
it, an edit included. So the writer decides when:

- The writer's connection registers a WAL hook (`Connection::wal_hook`, verified in the locked rusqlite revision `9d76035`
  behind the `hooks` feature, `Cargo.toml:36`; it takes `&mut Connection`). Registering a hook replaces SQLite's default
  hook, which is the automatic checkpoint (`sqlite3_wal_autocheckpoint` is itself a `sqlite3_wal_hook` call in the bundled
  `sqlite3.c`), so no pragma is needed. The hook runs only on a commit. It receives the frame count and, at 1000 frames or
  more (SQLite's default), marks a checkpoint as due. It does no SQL and always returns `Ok(())`, since an error becomes
  an SQLite error code.
- Read connections never commit a write, so they never run an automatic checkpoint, and they are closed by drop
  ([Design](#design)), so they never run the close's `TRUNCATE` either. The rebuild's connection has no hook; it ends with
  `wal_checkpoint(TRUNCATE)` and a close (`crates/catalog/src/rebuild.rs:119-120`).
- *When it runs.* A due checkpoint runs once, as `PRAGMA wal_checkpoint(PASSIVE)`, as soon as no edit is queued: ahead of
  the next service or job write, or at once if the queue is empty. Running it clears the mark, whatever it achieved; only
  a later commit can set the mark again. So an idle writer does not checkpoint in a loop, and a session of edits alone
  still checkpoints when the edits pause. An edit that arrives during the checkpoint waits for it. A passive checkpoint
  reports "busy" as a column of its result row, not as an error, so the writer reads the row.
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
  - Commits of a few frames each cross 1,000 frames with the queue empty. The commit after the checkpoint reports a frame
    count below 1,000 (the WAL restarted).
  - An edit-class write is queued when the count crosses 1,000. It returns before the checkpoint runs.

### Part 3, the read side

- **Point lane**: as in [Design](#design). Each request runs in one deferred read transaction (Part 1).
- **Listing lane**: the existing `Reader`, unchanged in behaviour; its results gain the revision of their snapshot.
- **R2 on the Qt side.** Every stored value gains its revision, the sparse map and the details slots are added, and the
  notes of reads in flight replace `note_turns_written`.
  - *The interim reply is applied as an ack.* Until Part 4 the GUI still waits for each command's reply (Part 1). From
    Part 3 that reply carries the snapshot and its revision, and the GUI applies it as R4's ack: to the stored layer and
    the sparse map, and into the notes. Without this, a read in flight during a rating would land over the new value,
    since the GUI's own update after the call has no revision. Part 1's reply carried no snapshot; this changes its
    payload.
  - *The ack replaces the GUI's code after the call.* Today that code changes the grid itself: `rotate` adds the turn to
    `pending_turns` (`gallery/edits.rs:36`), and the other calls edit the entry, forget the details, re-read the views
    and the roots, and emit `scanned()` after a removal. Left in place beside the ack, a turn would be applied twice and
    `scanned()` emitted twice. From Part 3 the ack does the data changes. What stays, now driven by the ack: the row
    signals; the re-plan when a turn changed the aspect (`gallery/edits.rs:41-43`); `bump_revision` (`:47`); and the
    operation's success resolve, which clears its failure notice (`resolved_part("edit-failed", ...)`, `edits.rs:31`,
    `:83`, `:105`; `resolved_part("views-failed", ...)`, `shell.rs:319`, `:327`; `resolved("session-not-saved")`,
    `shell.rs:497`). Without the resolve a failure notice would never clear after a later success. No QML caller reads
    a return value from these calls, so nothing else depends on them being synchronous.
  - *The synchronous re-reads go.* `fresh_entry` in `Grid::arrive` is removed: the reply applied as an ack, and the
    notes, cover the edits made while a list loaded. The one in the controller's `Command::RefreshTurned`
    (`desktop/src/gallery/edits.rs:58`) is removed too: the `Turned` set (below) asks the point lane, and
    `RefreshTurned` stays as a controller command that reads nothing, so recordings still replay.
  - *An empty slot.* The grid's selection has one slot. While its details are on the way, the selection's line shows what it
    shows today without details: the rating symbol alone. The full line needs the details' path and size
    (`gallery/items.rs:193`).
- **The bus and the watcher's route.** `App::open` creates the change bus and gives the `Database` the functions that
  publish `Change::Foreign` (R3a) and set the busy state (R5). The desktop subscribes its library pump to that bus at
  Ready, whether or not `library().start()` succeeds: today the pump exists only after it succeeds
  (`desktop/src/library.rs:105-122`), and it runs from `startHooks` after the first list (`Main.qml:752-757`). So no
  change is missed between the first list and the job's start, and none needs a catch-up refresh at the moment
  `Main.qml:750-751` defers the job to protect.
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
  - `Change::Foreign { revision: u64 }`, from the watcher's route (R3a): refresh everything.
  - *No event for a command.* The window that sent it gets the ack, which carries the new value and feeds R2's notes,
    and there is one window. So a rating never goes through the pump, and never causes a re-list (R4).
  - `Scanned`, `Imported`, `Round` and `Writes` keep their handling and stay fieldless: `Scanned` reloads the tree and the
    grid, and `Imported` re-lists, once per pass (`Main.qml:127-128`, `:255-265`; `crates/library/src/runner.rs`,
    `Imported`). Their reloads go through the listing lane, whose results carry their own revision. The job's per-batch
    commits publish nothing more.
  - `Progress` is unchanged.
- **Pumps.** A pump waits with `recv()` for its first event, then calls `take()`. It handles the event it received, then
  everything `take()` returns, in order, then the lag count. `take()` drains the queue, so its events are not discarded.
  `take()` alone does not wait, so it cannot be the wait. For the library pump a non-zero lag count is the R3 refresh; the
  notices pump logs it; the Yandex pump's events carry nothing (`desktop/src/yandex.rs:127-128`: its change is `()`, since
  a poll reads the whole state), so it discards them and the count.
  - *Today no pump reads the lag count.* The library pump takes one event per `recv()` and folds it into flags
    (`desktop/src/library.rs:114-122`, `pump.rs:51-64`); the notices pump does the same on its own thread
    (`notices.rs:260-277`); the Yandex pump discards what `take()` returns. `take()` is the only reader of the count, and
    reading it resets it (`events.rs:103-106`). The log panel's pump uses an unbounded channel and has no lag
    (`log_model.rs:124`). `pump::start`'s `recv: FnMut() -> Option<C>` changes to return a batch and the lag count, which
    changes the Yandex pump's call too (`yandex.rs:126-130`).
  - *Folding `Turned`.* The library pump's flags (`Changes`, under `Pending`, `desktop/src/library.rs:43-62`,
    `pump.rs:14-25`) gain a set of (file, revision). Above 256 files in one pass the set collapses to "refresh
    everything".
  - *The status.* The pump's thread reads `library().status()` when an event arrives and hands it to the Qt pass with the
    folded changes ([Inventory](#inventory)). It is built here, with the pump.
  - *The set's route to the grid.* Today the pump reaches the grid through a QML signal and the controller
    (`Main.qml:130`, `Event::FilesChanged`, the controller's `Command::RefreshTurned`, `gallery::post`;
    `crates/client/src/controller.rs:327`, `desktop/src/shell.rs:495`), a chain that carries no payload and is recorded for
    replay. The set does not go through it: the `Library` object hands it to the gallery model in Rust (`gallery::post`,
    which takes any closure), and the signal stays as it is.
- **Read-ahead.** When the viewer's open item or the grid's selection changes, the GUI queues point-lane reads for the
  slots that changed. Their results apply under R2. A details or entry read measured 0.08 to 0.12 ms (p50) and at most
  0.39 ms.
- **The point-lane conversions.** The reads the Inventory targets at the point lane move to it here
  ([Catalog reads](#catalog-reads-on-the-gui-thread--point-lane)).
- **Startup.** All of R9 is built here: the startup worker, the slot, `Startup`, the `Loader`, the busy function passed
  into `app::open` and its holder (R5), the failed open and the close before Ready. `app::open` and the start state's reads move to the
  worker, so the GUI thread opens no catalog, and Part 1's 5 s deadline on the open is replaced by the worker's stop.
- **The screenshot harness.** Its settle check (`Shell::pending`, `shell.rs:242-244`) gains "no point-lane read in
  flight", as `failed_files::loading()` already is there: a details read that lands late changes the status line. The
  goldens that rate and turn depend on it (`viewer-turn`, `viewer-rate`, `tools/compare_shots.py:113-114`).

### Part 4, the GUI stops waiting

Ratings, turns, border marks, saved views, the session and the folders are already commands (Part 1). Here the GUI stops
waiting for them: it sends, shows the pending set, and takes the ack when it is posted to the Qt thread (R4 to R7). An
add's probes run before the command is sent, on a probe thread (R7). A rescan is not a command (R7).

What the Qt side gains:

- the pending set, with the cap (R5). Coalescing (R6) can now happen, since the GUI no longer waits;
- every reader of an edited field reading the shown value (R4);
- the busy state's property and its transient toast (R5). The status bar's toast is the inbox's today (`Main.qml:486`),
  so this one needs a path of its own. `Library.busy` already exists and the harness reads it
  (`cpp/screenshot.cpp:105`), so the property has another name;
- the screenshot harness's settle check gains "the pending set is empty and no model command is posted"
  ([Shutdown](#shutdown));
- pending adds and removals in the library panel (R4): the dimmed row and the separate lines, "Rescan all" disabled
  while a removal is pending, and the empty-state texts while the first add is pending;
- the close's decision and the count of posted model commands ([Shutdown](#shutdown));
- the mark of a pending value. The grid and the viewer have no `Accessible` properties at all today (only `LogPanel`,
  `StatusBar` and `InboxPanel` do), and a row's roles (`GridRow.qml:16-28`) have none for "pending". Both are new: a
  pending role on the row and in the viewer, an accessible description from it, and the one 200 ms timer, which needs a timer of its own: the existing one is the
  `Shell`'s deadline (`deadline.rs`);
- `Grid.bordered`, which is maintained incrementally (`crates/client/src/grid.rs:323-342`), derived from both layers;
- the window's close and the session-end handler ([Shutdown](#shutdown)). The shutdown's steps themselves land with the
  part that builds each thread: the command thread's stop and the `Database`'s close in Part 1, the point lane's in
  Part 3, the pool's, the preferences writer's and the log thread's in Part 5.

### Part 5, the worker items

The file, lock and log work that the GUI thread still does moves off it, onto Part 5's pool. See
[Not the catalog](#not-the-catalog-but-on-the-gui-thread--worker-part-5). The session-end handler's wait on the
preferences writer is added here, with that writer.

### Tests for Parts 3 to 5

Parts 1 and 2 state their tests where they are described. These are the ones the later parts need:

- **R2** (Part 3), on `pg_client::Grid` and the desktop model, with results fed in a chosen order:
  - an older point read that arrives after a newer ack is refused;
  - an older listing applied after an ack keeps the acked entry, and the entry stays in the sparse map;
  - a file that turns while a listing containing it is in flight is asked for again when the listing lands;
  - a listing held under the viewer keeps the notes until it is applied;
  - a failed listing shows empty and changes no revision;
  - an applied listing asks again for each details slot older than it.
- **R3 and R3a** (Part 3): two causes inside 5 s give one refresh after the first; a commit by a second `Database` on the
  same file, standing in for another process, is reported once; this process's own commit is not reported; a foreign commit followed at once by an own
  commit is still reported.
- **The interim reply** (Part 3): a turn through the blocking reply is applied once, and `scanned()` fires once on a
  removal.
- **Part 4**, on the plain types ([Design](#design)) and the controller, with virtual time:
  - a pending rating shows at once and is marked after 200 ms; a failed command restores the stored value;
  - a second border toggle while the first is pending sends "unmark";
  - the cap refuses the 10,001st file command and not a session save;
  - the close's decision: a close with a sent command pending, or with a model command posted and not yet run, is
    refused and completes on the ack; a close with only an unsent add pending is accepted; a second close while waiting
    is refused without a flush; "quit anyway" makes the next close accepted; a file command that fails while waiting
    returns the decision to idle; a session save that fails while waiting lets the close complete; a posted closure that
    is dropped without running stops holding the close;
  - a probe result that arrives while closing sends nothing; after "keep working" it is sent;
  - while closing, the controller returns no session save; "keep working" clears the closing state, arms the session
    save, and fires again only the timers the flush drained: with no query waiting it makes no listing;
  - the save-now event returns the session save when only the query's timer is waiting, and leaves that timer and the
    layout's armed;
  - a close request when the decision is complete is accepted without a flush; a file command that fails after "quit
    anyway" does not take the close back;
  - session end: when idle it sends save-now and blocks; when waiting it blocks and sends nothing; when complete it does
    nothing.
- **Startup** (Part 3): session end before Ready does nothing; a close before Ready ends in the ordinary shutdown
  whatever the worker reports.
- **Part 5:** two preferences saves queued together leave the newer on disk; the log thread's lines are on disk after a
  normal exit and after a panic.

### Part 6, enforcement and visibility

- The blocking entry points move into the facade's `providers` module, and the facade's thread check is added, in every
  build (R8). `write`'s own check is built in Part 1.
- R10's moves to `pg-model`, the removal of `reexport.rs`, and its check: no `pub` item of `pg_app` names a `pg-catalog`
  type.
- Part 1's rule needs no check of its own: a read connection refuses a write, and a write function needs a transaction.
  A test opens a read connection and asserts that a write on it fails.

## Shutdown

**Closing waits for the edits under way.** The close itself sends commands: `onClosing` runs `shell.flush()`
(`desktop/qml/Main.qml:138`), which saves the session and the last file (`crates/client/src/controller.rs:509-535`), written
synchronously today (`desktop/src/shell.rs:496`). The flush also drains the timers: it fires the row-height save, turns a
query typed in the last 300 ms into a save with no listing, and drops a layout waiting on a resize
(`controller.rs:509-523`, `crates/client/src/controller/deadlines.rs:64-68`). That is right for a window that is going
away, and only for that.

**The close's decision** is one plain value with three states: *idle*, *waiting* and *complete*. `onClosing` reads it,
and nothing else decides whether the window may close.

- *What holds a close.* Two things: a command that has been sent and is neither acked nor failed, and an edit that has
  been dispatched but whose command is not yet made. Rate, turn and the border mark are made in a closure queued to the
  model (`shell.rs:477-494`, `gallery.rs:61-64`), so between the dispatch and that closure the pending set does not hold
  them yet. A count of such closures posted and not yet run stands beside the pending set; the screenshot harness's
  settle check reads it too. The count is a guard carried inside the closure and released when the closure is dropped,
  run or not. It is taken when the closure is built. `gallery::post` discards a closure when there is no model and
  ignores a failed queue (`gallery.rs:64-66`), and a live reload destroys the model; in each case the closure is dropped,
  which releases the guard. A count lowered only by a run could stay above zero for good. An add whose command has not been sent holds nothing: its probe may never return (R7).
- *A close request when idle.* `onClosing` flushes, as today, and then sends the controller a closing event, in that
  order: the flush's own session save must pass before the gate below shuts. If nothing holds the close, the decision is
  complete and the close is accepted. Otherwise it is waiting: the close is refused (`close.accepted = false`, which the
  code does not use today) and input is blocked.
- *A close request when waiting* (a second Alt+F4) is refused, with no flush.
- *A close request when complete* is accepted, with no flush. This is the close the window sends itself.
- *Waiting becomes complete* when nothing holds the close any more, or when the user chooses "quit anyway". The window
  then closes itself.
- *A file, view or folder command that fails while waiting takes the close back.* It sends the controller the same
  event "keep working" does. Its failure notice is then seen, which it would not be if the window closed over it.
- *A session save that fails does not.* It only stops holding the close. It is not a user's edit, and taking the close
  back for it would loop: the take-back arms the session save again, the next close's flush sends it, it fails within
  milliseconds, before "quit anyway" is ever shown, and the window could not be closed while the catalog refuses writes.
  Its notice is posted as today.
- *Complete is final.* Nothing takes a complete close back.

What holds while the decision is waiting:

- *The controller returns no session save.* The closing state is a flag inside the recorded controller, set and cleared
  only by its own events. The session save is re-armed a second after the current item changes (`controller.rs:292`),
  and a list that arrives changes it with no input at all (`apply`, `set_current_item`, `onCurrentChanged`,
  `Main.qml:137`). Three more paths return a session save with no timer (`Event::FolderSpelled`, `controller.rs:313-316`,
  when a tree result lands; the start gate with an applied view, `:359-363`; a reveal's delivery, `:495`). So the gate is
  where the controller's `update` returns `Command::SaveSession`: while closing, it returns none. What those paths would
  have saved is not a user's edit, and is not saved if the window closes.
- *Input is blocked* by an overlay like `ModalOverlay.qml`, but not that one as it is: it dismisses on Escape and on a
  click outside (`ModalOverlay.qml:22`, `:30`) and dims as soon as it opens. This one cannot be dismissed. The three
  application shortcuts bypass an overlay (`GlobalShortcuts.qml:24-41`), so the saving state is a `PanelKind`, which
  makes the controller swallow them; a `PanelKind` is recorded but not saved in the session.
- *The "saving" state shows only after 200 ms.* The flush sends the session save when one was waiting, and it commits in
  milliseconds. Shown at once, the state would flash on such a close. After 200 ms the overlay dims and shows two
  choices, "quit anyway" and "keep working".
- *A probe result that arrives is kept, not sent.* The add stays pending.
- The job and the write-back are still running, so a late turn is written.

**"Keep working" takes the close back.** Without it, a catalog held by the command-line tool for minutes would leave the
window blocked with only the choice that loses the edits. It is a controller event, and it undoes what the request did:

- the decision returns to idle, and the closing state and the saving panel are cleared; the pending commands stay
  pending;
- the session save is armed, since saves were suppressed meanwhile;
- the query's timer and the layout's are fired again, each only if the flush drained it. The controller notes that at
  the flush (`controller.rs:512-517`). The flush's other two, the row-height save and the session save, were carried out,
  not dropped, and are not repeated. A listing made without a waiting query would show from the top
  (`open_list(false)`, `controller.rs:155-156`, `:549-551`) and lose the scroll position and the selection, so none is
  made;
- a kept probe result is acted on now: the add's command is sent, or its refusal shown.

**When the window closes.** An add whose command was never sent is dropped, and the `Shell` logs it: nothing was written
for it. After "quit anyway", the commands still in the command thread's queue are dropped and logged by that thread
(step 1 below).

**The stated guarantee.** An edit the user made is lost at exit only in these cases: an add whose command had not been
sent when the window closed; the user chose "quit anyway"; the operating system ended the session before it was written
(below); or the process was killed. The first two are logged. At session end, what the command thread still holds is
logged, though the line may not reach the file, since the process can be killed right after; an unsent add, or an edit
whose model closure has not run, is not logged then, because no thread but the blocked one knows of it. A notice's dismiss, mark or retry is not a command (below).

**The operating system ending the session.** The app is a plain `QGuiApplication` (`cpp/gui_app.cpp:77`). At session end
Qt emits `commitDataRequest` on the GUI thread, inside its handler for the system's query, and closes no window, so
`onClosing` never runs. Windows can cancel a session end after this signal; Qt then does nothing. If it is not cancelled,
Qt quits the application (`qwindowscontext.cpp:1139-1176` in the Qt source). So the handler must leave the window able to
go on as before, and the shutdown must cope with a quit that never went through the close's decision.

- *Before Ready* the handler does nothing. There is nothing to save, and stopping the open would leave a cancelled session
  end with a window that cannot start.
- *While the close's decision is waiting* it sends nothing more: the close's flush has run.
- *When the decision is complete* it does nothing at all: the window is already closing, and after "quit anyway" there
  is nothing the user still wants written.
- *Otherwise* it sends the controller a save-now event, which is not the flush. It returns the session save, with the
  last-file save that goes with it (`controller.rs:532-535`), when the session's timer or the query's is waiting: the
  listing already holds a query typed in the last 300 ms (`controller.rs:301-304`), so it is saved, and its timer stays
  armed so it is still listed if the session goes on. It fires the row-height save. It leaves the layout's timer armed.
  It sets no closing state and drops no add.
- *Then, when the decision is waiting or idle,* it blocks on the command thread until its queue is empty and nothing is in progress, and
  on the preferences writer's drain (the last-file save is a preferences write, `shell.rs:99-102`), for at most 3 s
  together. It does not wait for the pending set: acks are posted to the Qt thread, which is the thread that is blocked.
  What the command thread still holds when the 3 s end, it logs.
- Blocking in that slot is permitted. The process may be killed right after, so the ordinary shutdown is not relied on.
  The 3 s is chosen to stay under the system's own limit, which is not checked ([Open](#open)).
- This wait is an accepted exception to the goal, and one named facade entry is exempt from R8's check for it.
- *The plumbing.* A direct connection on the application object calls an invokable of the current `Shell`
  synchronously, through the root's `shellCore`, as the screenshot harness already does for `pending`
  (`cpp/screenshot.cpp:82-84`). Only the connection is new.

**Not commands.** A notice's dismiss, mark or retry goes through the inbox, not the command thread
([Inventory](#inventory)). The inbox abandons what it cannot write within its 5 s drain, as it does today.

**Quits that skip the close's decision.**

- A session end that is not cancelled: Qt quits, the event loop returns, and the ordinary shutdown runs with whatever is
  left: what the 3 s wait did not see written, or everything queued if no wait ran.
- The screenshot harness calls `QCoreApplication::quit` and `exit` (`cpp/screenshot.cpp:174`, `:186`, `:211`), and its
  settle check does not see pending commands or the mark's timer (`screenshot.cpp:79-112`, `shell.rs:242-244`); the check
  gains "the pending set is empty and no model command is posted".
- The debug build's live reload deletes the root (`cpp/live_reload.cpp:95`); commands already sent still run on the
  command thread, and that is accepted for a debug build.

In the first two, step 1 below drops and logs what the command thread still holds. A live reload runs no shutdown. The
controller's core is built once per process, so a reload while the decision is waiting returns it to idle and clears
the closing state for the new window. The closing state is cleared only by its own event, and `ShellRust::default`
cannot dispatch one (`shell.rs:36-56`). So the `Shell` gains `cxx_qt::Initialize`, as `GalleryModel` has
(`gallery.rs:70`), and there dispatches the keep-working event when the controller is closing.

Today `crates/desktop/src/lib.rs:212-214` runs `app::stop()`, then `drop(engine)` (which joins the image providers' pools),
then `app::shutdown()`. Two things order the new steps. The writer must outlive everything that writes through it. And
each stop below raises the stop that its thread's `write` calls carry, so a call that is waiting ends with `Stopped`, at once or, for the job
and the write-back, when their 4 s grace has passed; a transaction that has begun runs to its commit.

1. Stop the command thread. Its queue is empty, or the quit did not wait for it (the user chose "quit anyway", the
   session ended, or the harness quit) and what is left is dropped and logged.
2. `app::stop()`, as today (`crates/app/src/app.rs:148-154`): the Yandex search stops first (up to 2 s), then the listing
   lane, then the library (the write-back, then the job, whose quit flag is raised here). `Library::stop` holds the
   write-back's slot mutex while it joins the write-back (`crates/app/src/library.rs:166-171`). Nothing that holds that
   mutex calls `write`: the command thread, which takes it to wake the write-back, has stopped. The pump's `status()`
   takes it too, and `tend` under it can restart a dead write-back (`library.rs:125`); neither writes to the catalog
   while holding it.
3. `drop(engine)`: the providers finish, and their pools join (`crates/desktop/cpp/providers.cpp:131-136`). The providers
   only read, so they may run through steps 1 and 2. The pumps' threads are detached (`pump.rs`), and their passes are
   queued to a Qt thread that no longer runs its loop, so they do nothing more.
4. Stop the point lane and Part 5's pool. The point lane answers its queued requests with an error. A probe thread that
   has not returned is abandoned.
5. `app::shutdown` (`crates/app/src/app.rs:160-177`): the thumbnail cache closes (`thumbs.close()`, `:164`); the event
   forwarder is joined (`:166-170`); the preferences writer drains and closes (Part 5); the inbox closes
   (`crates/app/src/inbox.rs:454-463`), abandoning what it cannot write within its 5 s drain.
6. The `Database` closes. The watcher stops first, so it calls nothing that has closed. The writer finishes a
   transaction that is still running: that is a job that was abandoned after its 5 s stop wait
   (`crates/library/src/runner.rs`, `stop`; `crates/app/src/library.rs:172`), inside a long closure or beginning a write
   it found the writer free for. Then it runs
   `wal_checkpoint(TRUNCATE)` and closes its connection. `TRUNCATE` waits for readers through the busy handler, which
   gives up after 5 s when no call is current (Design), as the timeout does today. The read connections belong to their threads and are dropped as
   those end, before this step. An abandoned job thread still holds its own; then the writer's is not the last
   connection, and the WAL file stays until the next open, which is safe. A `write` it makes later returns `Stopped`.
7. The log thread flushes and ends last (Part 5).

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
  mutex that no thread holds across I/O, which the Goal accepts:
  - `inbox().subscribe()` takes the `first` mutex (`inbox.rs:448-450`), and the Yandex `search().subscribe()`
    (`desktop/src/yandex.rs:120`) takes its bus's mutex;
  - `inbox().idle()` takes `shared.work` (`inbox.rs:467`), which the inbox thread holds only around its condvar wait and its
    queue bookkeeping (`:776-784`, `:879-890`, `:914`);
  - `inbox().listing()` takes a separate `listing` mutex (`:443`), which the inbox thread holds only for a pointer swap
    (`:959`).

  The inbox thread holds none of them across SQL, so these stay.
- **In memory, but with a lock the GUI waits on, and a stop of the running pass**. Part 5 moves these, except where
  Part 1 is named:
  - `library().set_thumb_height` (`crates/app/src/library.rs:252`), which takes a mutex in `HeightCell::set` and wakes the
    job;
  - `library().save_crops` (`gallery/edits.rs:124`), which takes the job's queue mutex and wakes it
    (`crates/library/src/runner.rs`, `save_crops`);
  - `library().wake_job()` in `on_pass`'s `turned` branch (`desktop/src/library.rs:142-144`): `request_stop` and `wake`
    (`runner.rs`, `wake` and `wake_up`);
  - `Runner::scan` after `rescan` (`runner.rs`, by symbol): the queue mutex and `request_stop`. The one after
    `add_folder` moves to the command thread in Part 1;
  - `wake_writer` inside `library().turn` (`crates/app/src/library.rs:176-180`, `:339`): the write-back's slot mutex and `tend`,
    the same pair listed for `status()` below. It moves to the command thread in Part 1.

### Catalog reads on the GUI thread → point lane

| Site | Call | Reached from | Target |
| --- | --- | --- | --- |
| `gallery/state.rs` `with_details` | `reading().details` | `gallery/model.rs` `refresh_viewed` (from `set_viewing_holding` and `bump_revision`) and `refresh_derived` (`model.rs:264-272`); the `info` and `border_rejected` callers are Rust, not QML | point lane, into a details slot (Design) |
| `gallery/state.rs` `fresh_entry` | `reading().entry` | `Grid::arrive` (`crates/client/src/grid.rs:289`, for edited ids) and `refresh_turned` (`gallery/edits.rs`) | removed for `arrive` in Part 3 (the reply applied as an ack, and R2's notes, below); the point lane's re-read at `Turned` for `refresh_turned` |
| `gallery/state.rs` memo | two entries (`state.rs:129-130`) | `with_details` | replaced by the details slots (Design, Part 3) |
| `animation.rs` `restart` | `catalogs().file_path` | the viewer starting an animation | point lane |
| `video_source.rs` `play` | `catalogs().file_path` | the viewer starting a video | point lane |
| `yandex.rs` `search_file` | `catalogs().file_path` | the Yandex panel's search of a file | point lane |
| `library.rs` `load_roots` | `library().roots()` | `start`, `add_path`, `remove_folder` | point lane for `start`; the command's ack after an add or a removal (R4) |
| `library.rs` `rescan` | the roots read (`crates/app/src/library.rs:327-329`) | `rescan` (`crates/desktop/src/library.rs:240`), before it queues the scans | point lane, then the library job queues the scans (R7) |
| `shell.rs` `view_names` | `views().all()` | the saved views' list, after each save and delete, and at startup (`shell.rs:48`) | the command's ack after a save or delete (R4); the startup worker's slot at startup (R9) |
| `shell.rs` `apply_view` | `views().view` | the views menu, called synchronously from QML | an in-memory lookup in the saved views the GUI holds (R4) |
| `controller.rs` `Core::new` | `views().listing()`, `views().view`, `prefs().get()` | `Shell`'s construction during QML load, before the first frame | read on the startup worker and taken from the slot (R9) |

**The listing's `arrive`.** `Grid::arrive` reloads the edited ids (`crates/client/src/grid.rs:282-292`) through the synchronous
`fresh_entry`, for the ids the GUI has edited while the list loaded. The design needs no such reload. The listing's rows go to
the stored layer under R2 with the listing's revision. An edit made while the list loaded was acked while the read was
in flight (from Part 3 the blocking reply is applied as an ack; from Part 4 it may also still be pending, and is then
drawn over the new rows). The acked entry is in the sparse map at its higher revision, so the older listing's row for it
is refused, and R2's notes ask for its details again.

### Catalog writes on the GUI thread → commands

| Site | Call | What it writes | Target |
| --- | --- | --- | --- |
| `gallery/edits.rs` `rotate` | `library().turn` | the `file_writes` turn row, then `wake_writer` (the write-back's slot mutex and `tend`); the image write happens on the write-back thread | command (R6: turns add up); the command thread wakes the write-back after the commit |
| `gallery/edits.rs` `set_rating` | `edit().rate` | `set_rating` (and its write-back) | command (R6: last value) |
| `gallery/edits.rs` `set_border_marked` | `edit().set_border_mark` | `border_marks` | command (R6: last value) |
| `shell.rs` `save_view`, `delete_view` | `views().save`, `views().delete` | `views` | command |
| `shell.rs` `execute` (`SaveSession`) | `views().save_listing` | `session` | command |
| `library.rs` `add_path` | `library().add_folder` | normalize and `check_folder`, then `add_root`, then `Runner::scan` | pending from the click; the probes on a probe thread, then one command for `add_root`; the command thread queues the scan after the commit (R7) |
| `library.rs` `remove_folder` | `library().remove_folder` | a roots read in its own statement (`crates/app/src/library.rs:315-322`), then `remove_root` (one transaction; sets `missing = 1` over the folder, `crates/catalog/src/roots.rs:58-72`), then `Runner::folder_removed` with the root's path (the job's queue and busy mutexes, and a stop request) | one command, one transaction, without the outer read; the write returns the path, and the command thread calls `folder_removed` after the commit (R7) |

`add_path` reaches `check_folder` on the GUI thread today: `add_path` (`desktop/src/library.rs:211-212`) calls
`app::get().library().add_folder` (`crates/app/src/library.rs:298`), which calls `normalize_path`
(`crates/library/src/folders.rs:29-31`) and `check_folder`. On Windows `check_ntfs` (`crates/scan/src/refusal.rs:39-66`) makes the
volume lookup, the drive-type query, a handle open, the reparse-point test, the final-path call and the file-system query
(`crates/platform/src/dir.rs:117` onward). All of that is file or volume I/O, and it moves to a probe thread (R7). Today
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
  thread.
- **`set_thumb_height`**: takes a mutex and stops a running pass (`crates/library/src/runner.rs:57-76`, which wakes the job
  after releasing its own mutex; the stop is `request_stop`, `crates/jobs/src/control.rs:117-121`, which takes the `paused`
  mutex). The GUI keeps its own copy of the height, because the layout uses it; a worker is told of each change.
  Target: a request to Part 5's pool.
- **`save_crops`, `wake_job` in `on_pass`, `Runner::scan`**: the same job mutexes and the same stop of the running pass
  (see [Already off the GUI thread](#already-off-the-gui-thread)). Target: requests to the pool, the rescan's
  `Runner::scan` among them. `Runner::scan` after an add and `Runner::folder_removed` after a removal run on the command
  thread, after the commit (Part 1).
- **The animation player's join.** Dropping a `Player` joins its thread (`crates/render/src/player.rs:55-65`). `restart`
  drops one on every change of an animated item (`desktop/src/animation.rs:54`), and that thread may be inside
  `std::fs::read`. Target: the old player is handed to Part 5's pool to drop.
- **The clipboard.** `setText` (`desktop/src/notices.rs:441`, `cpp/clipboard.cpp:12-13`) runs on the GUI thread, where Qt
  requires it. Whether it can wait on another process is not checked ([Open](#open)). Accepted (see the Goal).
- **The Yandex search.** `search().start` (`desktop/src/yandex.rs:102`, from `search_file` and `search_command_line`) takes
  the search's `shared` mutex, publishes and spawns a thread (`crates/app/src/yandex.rs:421-436`). `search().poll()` runs
  on every pump pass (`desktop/src/yandex.rs:263`) and takes the same mutex and the pictures' `take_dead`
  (`app/src/yandex.rs:485-489`). How long the search's workers hold `shared` is not checked ([Open](#open)). Target: the
  start is a request to the pool, and the search pushes its state, as the library's status does below.
- **The log panel.** `pg_log::subscribe()` (`desktop/src/log_model.rs:112-113`) takes the logger's `ring` mutex and clones
  the kept records (`crates/log/src/lib.rs:299-305`). The file read is already on its own thread. Target: the subscribe
  moves to that thread.
- **`library().status()`** in `refresh_progress` and `refresh_writes` (`desktop/src/library.rs:166, 193`): takes the write-back's
  slot mutex (`crates/app/src/library.rs:211-213`), and `tend` restarts a dead write-back thread under it (`:125`). Target: the
  library pump's thread reads the status when an event arrives and hands it to the Qt pass with the folded changes. It is
  not a `Change`, which stays `Copy` and small. The GUI keeps the last status it was handed.
- **`set_paused`, `is_paused`**: take the job's pause and queue mutexes (`crates/library/src/runner.rs`, by symbol).
  Target: requests to the pool, with their state pushed back.
- **Logging.** Every `pg_log` call takes the ring lock (`crates/log/src/lib.rs:269`) and the logger's `writing` lock, and
  appends synchronously: it stats the file, checks that it exists, and opens it, for each line
  (`crates/log/src/lib.rs:240-249`, `crates/log/src/file.rs:53-63`). Target: a channel to a log thread, flushed on exit and on
  panic. A crash can lose the last lines, which the plan accepts for the GUI's own log calls. `Inbox::post`'s log line goes
  the same way.
- **Video.** `video_player.rs:14-16` calls `File::open` (`crates/platform/src/file.rs:164-166`, the Windows variant; the other
  variant is not checked). `cpp/video_source.cpp:47-60` then opens that handle as a `QFile` (`_open_osfhandle`,
  `QFile::open(fd)`), also on the GUI thread. `Player::start` (`animation.rs:68`) reads its file on its own thread
  (`crates/render/src/player.rs:72`), so it is not a GUI call. Target for the video: the pool opens the file and hands the handle to the GUI.
- **The native picker's modal dialog** (`crates/desktop/src/picker.rs:85`): the user waits on it, and its own event loop runs.
  Accepted. The non-native picker's folder listing (`picker.rs:102` and `:170`, Linux only) is a request to the pool.
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
The library job and the write-back were never started, so no image file was written. Debug build unless marked release.

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

Only one stall of 100 ms or more was seen per run, at the first 1000-frame checkpoint. The later checkpoints (about 20 of the
21 in a 3,000-turn run, at roughly 7 frames per turn) were not timed separately. So the data show that this checkpoint stalled, and
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
spent blocked on the import's lock (about 12 s of the debug run, the one 12.4 s flush, and about 9 s of the release run, its four flushes). An unblocked iteration
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

- **Checkpoints, counted. This gates Part 2b.** Count the checkpoints in a 3,000-turn run, and time each one. The turn
  timings already show that none but the first cost 100 ms or more. Not known: whether the later ones ran, what each
  cost, and what the first one pays for.
- **The cost of "refresh everything" on the GUI thread.** A full layout pass follows each listing; it is not measured, and
  it sets how far apart refreshes must be (R3).
- **The hand-off to the writer.** Each write now crosses to another thread and back. Its cost per transaction, and for
  about 250 catalog tests that each gain a file and a writer thread, is not measured (Part 1).
- **Session end.** Whether `commitDataRequest` reaches the app in practice, and how long the system allows the handler,
  is not checked ([Shutdown](#shutdown)).
- **How long the job's transactions hold the writer.** The scan batch (10,000 entries, or a whole larger folder), the
  borders batch, the pairing tiers and `mark_missing`, and the preparation's marks run by another process are not
  measured (Part 1). They decide how long an edit can stay pending. A scan batch that is a whole folder has no cap.
  `remove_root` is measured in debug only.
- **Removing a folder in shorter transactions.** With one writer, the order of the removal's steps is the queue's order.
  Whether that makes a split safe against a scan or a re-add between the steps is not designed (R7).
- **A neighbour that changed during the import's matching.** Whether a pair with an already-indexed file that changed
  between the matching and the commit should be re-checked inside the write, and whether today's import has the same
  window, is not traced (Part 2a).
- **The inbox's `Store` trait.** What the inbox does is decided: each step is its own service write, with the drain as
  its deadline (Part 1). How the trait (`crates/app/src/inbox.rs:759`), which has the busy loop built in today, is
  reshaped for that is left to the implementation.
- **Chosen values, not measured.** The 200 ms before a pending mark or the "saving" state shows (R4,
  [Shutdown](#shutdown)); the 5 s between refreshes (R3); the 10 s before the busy state (R5); the 10,000 cap (R5); the
  256-file collapse (Part 3); the 3 s wait at session end ([Shutdown](#shutdown)); the watcher's one second (R3a); the
  4 s grace on the job's and the write-back's stops (Design).
- **Startup with the `Loader`.** Today the QML load overlaps the GPU device's creation, and `gpu_thread` is joined after
  it (`crates/desktop/src/lib.rs:194-197`). With a trivial root, the heavy load runs later, with the placeholder showing.
  Whether startup gets slower, and by how much, is not measured (R9).
- **Cold-cache reads: not established.** Reading 2.9 GB did not evict the OS cache on this machine (64 GB of RAM, 21.7 GB free).
  Evicting it needs far more reads than a probe should do, or a reboot. Until then, the point lane's cost is the warm figure,
  about 0.02 to 0.05 ms for a `file_path` read.
- **Rescan's scans.** `rescan` was not measured with the scans run: with the library not started, it only reads the roots.
- **Startup and shutdown durations.** The `gpu_thread` join, the pipewire probe, the log setup, `app::open` and the start
  state's reads on the worker, `~Provider`'s wait for the pool (`crates/desktop/cpp/providers.cpp:135`), the Yandex stop
  (`STOP_WAIT`, 2 s, `crates/app/src/yandex.rs:102`), the job's stop (`JOB_STOP_WAIT`, 5 s, `crates/app/src/library.rs:32`),
  and the catalog close (`wal_checkpoint(TRUNCATE)`). Their durations decide whether the shutdown bound in
  [Decisions](#decisions) is typical, and how long the placeholder shows. Not measured: they need the app launched and
  closed with timing.
- **Shutdown waits outside the bound.** `~Provider`'s `waitForDone` has no bound (`crates/desktop/cpp/providers.cpp:135`),
  and a Yandex pool job can be in a network fetch when it is called (`crates/desktop/src/providers.rs:99`).
  The write-back's join (`crates/app/src/library.rs:166-171`) has no bound either: its waiting `write` calls end within the
  4 s grace, but an image write in progress does not. The join comes before the job's quit is raised, so the two graces
  can follow one another. An abandoned job's transaction, which the writer finishes before it closes,
  has none. `thumbs.close()` (`crates/app/src/app.rs:164`), the forwarder join, the preferences writer's drain, the
  command thread's, the point lane's, the pool's and the watcher's joins, the writer's `TRUNCATE` and the log thread's
  flush are not in the bound, and are not measured.
- **Measurement notes not resolved.** The first pass's release batch total is lower than the second's and is not
  explained. The catalog copy had "no pending files", yet the import runs had 500 and 2,000; how those were made pending
  is not recorded. `File::open`'s non-Windows variant is not checked.
- **Pairing shares.** The second pass's pairing shares come from a deleted probe and are not re-checked. The batch-matching cost
  of Part 2a is not measured.
- **Unchecked sites.** How long the Yandex search's workers hold `shared`; whether the clipboard's `setText` can wait on
  another process; whether the import pipeline delivers a batch in item order (Part 2a needs only the batch's own order,
  which it has).
- **A rescan beside a pending removal.** "Rescan all" is disabled while a removal is pending (R4). What a scan writes
  when it begins for a folder that is no longer a root is not traced.
- **Designed, not built:** Parts 1 to 6, with their tests as given in Parts 1 and 2 and under "Tests for Parts 3 to 5".
- **Verified against the code:**
  - the write-back has the file id at `Turned` (Part 3);
  - the rebuild ends with `wal_checkpoint(TRUNCATE)` and a close (`rebuild.rs:119-120`), and needs the exclusive hold;
  - the inbox's `idle()` and `listing()` take different mutexes, and the inbox thread holds neither across SQL
    ([Inventory](#inventory));
  - rusqlite's `busy_handler` and `wal_hook` take closures at the locked revision (`src/busy.rs:55-57`,
    `src/hooks/mod.rs:389-391`), `total_changes()` is exposed (`src/lib.rs:1064`), and the busy handler is invoked for
    `BEGIN IMMEDIATE`;
  - the bridge objects are QML elements built through `Default`, so the app cannot be passed to them (R9);
  - the keeper is only the prepared connection held open, so the writer's connection can take its place;
  - no read path writes: none uses a temp table, `ANALYZE` or a lazy insert, and `Catalog::open` writes nothing once the
    file is in WAL mode. So `query_only` breaks no read;
  - no transaction outlives one flush, and nothing reads its own uncommitted writes across transactions, so each can be
    one `write` call; the flush closures capture nothing that is not `Send` apart from the catalog handle;
  - `unsafe` is allowed in the workspace (`undocumented_unsafe_blocks = "warn"`, `Cargo.toml:65`), and `pg-catalog` holds no
    mutex a panic could poison;
  - no lock-order cycle among the write-back's slot, the job's queue and busy mutexes and the inbox's locks;
  - a connection's own commit does not move its `data_version`, and a passive checkpoint on a hooked connection is safe;
  - the thumbnail pass's only catalog writes are the failure ledger's;
  - an entry's `rotation` is `files.rotation` and its `pending_turns` is the queued `file_writes` value;
  - `Player::start` reads its file on its own thread (`crates/render/src/player.rs:72`), and `requestImageResponse` runs on
    Qt's pixmap-reader thread (`qquickpixmapcache.cpp:1000-1003` in the Qt source);
  - `pg_client::Grid` is outside the recorded controller, `gallery::post` takes any closure, and a `Loader` parents its
    item, so the screenshot harness still finds the objects;
  - there is a single window, no window geometry is persisted, and there is no undo and no multi-select edit.

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
- **Every `write` call carries its caller's stop, which ends it only while it waits** (Design, R5). A waiting call ends
  when its thread is told to stop, after its grace if it has one, so each stop in the shutdown returns, whatever another process holds. A closure that
  has begun is never cut: its transaction would be half done. A stop is a polled predicate, because whoever raises it
  cannot notify the writer's queue.
- **The job's write stop is a new quit flag, not `Control::stop`** (Design). `Control::stop` is raised by every new
  request and means "commit what you have"; the passes write after it is raised.
- **The job's and the write-back's stops have a 4 s grace** (Design). An instant stop would drop the job's final flush
  and the lease's release whenever the writer is briefly busy; today those get the 5 s busy timeout. A lease left
  unreleased makes a restart within 60 s wait on its own lease. It is 4 s, not 5, so that a write that gives up returns
  before the runner's 5 s wait for the job ends.
- **An add's command is sent by the Qt thread, when the probe reports** (R7). The probe thread cannot know that the
  window is closing; sent from there, an add could be written after the window decided to close without it.
- **An unsent add does not hold a close, and is dropped only when the window closes** ([Shutdown](#shutdown)). Its probe
  may never return. Dropped at the request, it would be lost even if the user then chose to keep working.
- **A panic that no caller catches shows its box on the writer's thread** (Design). Every write waits behind it until it is
  dismissed; the process does not end, and the writer then goes on. Accepted.
- **The close has one decision, with three states, and `onClosing` only reads it** ([Shutdown](#shutdown)). Deciding
  from the pending set at each close made the window's own second close be refused, so "quit anyway" never closed it.
- **An edit dispatched but not yet made into a command holds the close** ([Shutdown](#shutdown)). Rate, turn and the
  border mark are made in a closure queued to the model; a close between the two would otherwise be accepted and the
  command dropped.
- **A file, view or folder command that fails while the window waits to close takes the close back; a failed session
  save does not** ([Shutdown](#shutdown)). The failure notice of an edit is then seen; closing over it would hide that
  the edit did not happen. A session save is not an edit, and taking the close back for it would loop while the catalog
  refuses writes.
- **The count of posted model commands is a guard released on drop** ([Shutdown](#shutdown)). A posted closure may never
  run, and a count lowered only by a run would then hold every later close.
- **A close can be taken back with "keep working", which undoes what the request did** ([Shutdown](#shutdown)). Without it
  a long foreign lock leaves only the choice that loses the edits. It arms the session save and fires again only the
  timers the flush drained: a listing made with no query waiting would lose the scroll position and the selection.
- **Only `onClosing` flushes and sets the closing state; session end sends a save-now event** ([Shutdown](#shutdown)). A
  session end can be cancelled, and the window must then go on as before. The flush drops the query's and the layout's
  timers, which is right only for a window that is going away. The save-now event saves the session when the session's
  timer or the query's is waiting, so a query typed just before is not lost. Before Ready the handler does nothing.
- **A stopped open is not a failed open** (R9). A close before Ready stops the open; a fatal box for that would be wrong.
- **The file-lock wait at open takes the open's stop; later read connections never wait** (R9). The `Database` holds the
  shared hold for its life, so nothing can take the exclusive hold under it.
- **While closing, the controller returns no session save** ([Shutdown](#shutdown)). Gating only the timer's arming
  misses three paths that return one directly.
- **The roots list stays the stored one; pending adds and removals are drawn beside it** (R4). The panel's rows index the
  stored roots, so a list changed by pending items could send a removal to the wrong folder.
- **`Stopped` is not a failure** (Part 1). The write-back counts no attempt and sets no error for it, and the job posts
  no notice: the pass ends as a preemption does.
- **Every other connection is `query_only`, closed by drop, and a write function takes a transaction** (Part 1). The rule
  is enforced by SQLite and by the types, so it needs no check that greps the source. Closing by drop matters because
  `query_only` does not stop a checkpoint.
- **The `Database` is the only way the app, the tool and the tests open a catalog; the thumbnail cache's attach is the one
  exception** (Design). The cache and the catalog cannot see each other in the crate layers, and the attach only reads.
- **Tests use catalog files, not `:memory:`** (Part 1). A `:memory:` database has one connection. A single-connection mode
  would be product code that only tests use.
- **The preparation's writes run on the writer** (Part 1). They change values a running window shows, so they must bump
  the revision.
- **The scan's lease is released by a call, and its heartbeat rolls a batch back when the lease is lost** (Part 1). A
  write in a destructor could block during an unwind. A job write can now wait long enough to lose the lease.
- **A closure may borrow its caller's state, through one documented `unsafe` block** (Design). The flushes borrow
  pass-local state today, and the caller blocks until the closure has run or been withdrawn. The alternative is to
  restructure every flush to own its data.
- **A panic in a closure is resumed on the caller's thread, and the writer keeps its connection** (Design). The
  transaction's drop has rolled back. Reopening would add a failure path for nothing. The panic hook's "will be caught"
  marker moves to `pg-platform`, and each call carries its caller's value of it, so a caller that does not catch still
  gets the fatal box.
- **The writer serves edits first, then the write-back and the inbox, then the job, and never interrupts** (Design). An
  edit is what the user is watching; a job write is what can wait. The job is not starved: edits come at a person's
  pace and service writes are small.
- **A transaction holds only SQL, and decides inside itself** (Design, Part 2a). Anything computed inside it keeps every
  other write waiting, and anything read before it, on another connection, may be stale.
- **One catalog-wide revision, bumped once at commit, and only when the transaction changed something** (Part 1). A sequence
  from the GUI's commands alone cannot order the import, scan and removal writes, and a read must be able to say which
  writes it includes. An unconditional bump would turn empty commits into writes. Migrations bump explicitly, because
  `total_changes` does not count DDL.
- **Part 1 includes the command thread, with the GUI still waiting for each reply** (Build order). Part 1 makes the pool
  read-only, so the GUI's writes need their new path at once. The command's 5 s deadline keeps today's behaviour, and the
  GUI waits for a definite reply, so it never reports a failure for a write that then commits.
- **Reads never wait; the point lane never drops a request** (R1, Part 3). A read that waited on a pending write would tie the
  point lane to the writer. A `Reader`-style replacement would drop results.
- **The GUI holds stored values and pending commands as two layers; the stored values stay where they live, and the
  pending set is one** (Design, R1, R4). A read never has to be dropped or merged because an edit is pending, and a
  failed command needs no read to undo it. One set gives the cap its count, and the close's decision and the harness one place to look, beside the count of posted
  model commands.
- **Every stored value carries its revision and only moves forward; a change noted while a read was in flight is read
  again** (R2). That covers roots, views and details as well as entries. A revision check alone loses a file that changed
  while a listing containing it was loading. The notes ask for a read; they never refuse a result. A new listing removes
  from the sparse map only the entries it is at least as new as.
- **An ack carries what the catalog holds, read inside the command's transaction** (R4). The stored layer is refreshed in
  one snapshot at the ack's revision.
- **A command publishes no event** (Part 3). The window that sent it has the ack, and there is one window. So a rating
  never goes through the pump and never causes a re-list.
- **A pending turn does not turn the picture; the image key follows the stored layer** (R4). A key ahead of the catalog
  makes the providers serve, and Qt cache, unturned pixels under the turned key. The picture turns at the ack, as it
  does today after the queue write commits.
- **A pending value is marked after 200 ms, not at once** (R4). An ack normally lands within a loop turn or two, and a
  mark at once would flicker on every edit. The cost is that a new value shows unmarked for up to 200 ms.
- **The point lane has its own connection and keeps its page cache**, opened at startup, so the GUI never opens one.
- **Coalescing is per file and operation, on the command thread** (R6). Per file alone drops turns, because a turn is a
  delta.
- **The pending set is capped at 10,000 file commands, on the Qt thread** (R5). The GUI is the only place that knows the
  count when a command is made. The value is chosen, not measured.
- **The import computes its pairs outside the write transaction, stores only the pairs of stored files, and inserts into the
  index only after commit** (Part 2a). The reason is the hold: pairing was 76 to 92% of it, and it would now be the
  writer's. Today's import already skips pairing for a rejected file and has no index rollback, so this fixes no
  correctness fault. A failed commit leaves the index unchanged.
- **The writer waits on another process through a busy handler that gives up on the call's stop** (R5). No write is lost
  to the busy timeout, and no caller has a busy loop of its own: a write ends unwritten only when its own stop is raised.
- **The busy state shows after 10 s** (R5). It is a QML property, not a database notice, since a database notice is itself
  held by the same lock.
- **A checkpoint, if it must leave the commit path, is run by the writer, once per due mark, when no edit is waiting**
  (Part 2b). It is not a thread of its own. It bounds the WAL no better than the automatic checkpoint does: a long-held
  reader pins both. Part 2b is built only if the counted measurement shows that checkpoints stall as a rule.
- **The checkpoint trigger is the WAL frame count, through the WAL hook, not the file size** (Part 2b). The file keeps its size
  after a checkpoint, so a size test would fire on every commit.
- **Removing a folder is one transaction, and returns the removed root's path** (R7). The measured hold is 2.3 to 3.6 s
  (debug, one machine). Every write queued during it waits and none fails. The path is what stops the scans under the
  folder. Splitting the transaction is open.
- **An add is pending from the click, and its probes run on a thread of their own before the command is sent** (R7). On
  the writer, the command thread or any shared pool, a dead drive would hold everything behind it. Pending from the click
  means the user sees it is under way.
- **A rescan is not a catalog write** (R7). It reads the roots and queues the scans in the library job.
- **Another process's write is a stored revision this process has not seen, checked by the writer in every transaction
  and by a watcher once a second** (R3a). `data_version` also moves on this process's own commits. The watcher alone
  misses a foreign write that an own commit follows before it looks; the writer alone misses one while it is idle. A
  commit counter keeps the watcher from taking an own commit for a foreign one.
- **A refresh of everything is spaced 5 s apart** (R3). A foreign pass commits every second or two, and each
  refresh is a full listing and a full layout pass.
- **The inbox keeps its own `data_version` comparison** (R3a). It also tells the inbox of this process's ledger writes.
- **The command-line tool keeps writing the catalog itself.** The alternative is one process owning the file, with the tool
  sending its work to a running window. That would remove R3a and cross-process waits, at the cost of a protocol between
  the two. Two processes on one WAL database is a supported arrangement, so it is not taken.
- **A lagging subscriber and a foreign write refresh one fixed list** (R3). The list includes the folder tree, the facets
  and the status lines, which a re-read of the grid alone would leave stale.
- **The library pump subscribes at Ready, whether or not the library starts** (Part 3). Edits work without the job, and a
  change published before the pump subscribed would be missed.
- **A pump waits with `recv()`, drains with `take()`, and reads the lag count** (Part 3). `take()` is the only reader of
  the count, and no pump reads it today.
- **Per-file changes reach the grid in Rust, not through the QML signal** (Part 3). The signal chain carries no payload
  and is recorded for replay.
- **No second cache of the catalog with its own invalidation** (Qt-side values). A stored value is what one read or one ack
  returned. The details memo is replaced by four slots.
- **Preferences: the value is read from memory, and the file is written by the preferences writer, which drains at
  shutdown** (Part 5). A save no longer blocks the GUI's reads, and queued saves cannot land out of order.
- **The log is written by a log thread, flushed on exit and on panic** (Part 5). Every log call opens the file today. A
  crash can lose the last lines, which is accepted for the GUI's own calls.
- **Providers use `Images` and the catalog pool on their own threads, not the point lane** (Design). The point lane is not
  a bottleneck for them.
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
- **A close before Ready keeps the window until the worker ends** (R9). A window that vanished while the process still
  held the file would make the next launch fail to open it.
- **Closing waits for the commands already sent and the edits already dispatched, with "quit anyway" and "keep working"** ([Shutdown](#shutdown)). A drain
  with a deadline after the window has gone would lose edits silently. The close's own session save is one of those
  commands. Input is blocked meanwhile, and the controller returns no further session save, so nothing adds a command
  while the window waits. The "saving" state shows only after 200 ms, or it would flash on a close that has a save to
  make.
- **At session end the GUI thread blocks up to 3 s on the command thread and the preferences writer**
  ([Shutdown](#shutdown)). Qt closes no window then. If the session end is not cancelled, Qt quits and the
  ordinary shutdown runs with whatever the wait left. It waits on those threads, not on the pending set, because acks
  are posted to the thread that is blocked.
- **A `Turned` event names its file and revision; more than 256 in one pass become a refresh of everything** (Part 3).
  The grid re-reads only the files that turned. A lost `Toast` is dropped: the window shows only the last one of a pass.
- **Accepted on the GUI thread:** the clipboard, which Qt requires there; mutexes that nothing holds across I/O; and, in
  a debug build, live reload, which may drop the window with commands still running.
- **The shutdown joins run on the main thread after the window has closed.** No UI remains to block. The bounds for the listed
  waits are: the Yandex stop 2 s, the job stop 5 s, and the inbox drain 5 s. Together they are about 12 s. Excluded, and
  unmeasured ([Open](#open)): the listing lane's stop, `~Provider`'s unbounded wait, the write-back's join, an abandoned job's transaction,
  `thumbs.close()`, the forwarder, the preferences writer's drain, the command thread's, the point lane's, the pool's and the
  watcher's joins, the writer's `TRUNCATE` (which gives up after 5 s) and the log thread's flush. The bound is accepted for the listed waits only.
- **The details slots cover the viewer's item, its previous and next, and the grid's selection** (Design, Part 3). Arrow
  navigation in the viewer needs the neighbours ready, and the grid renders the selection's line from details. A read
  measured at most 0.39 ms.
- **A pending edit is marked by dimming, with its pending state exposed to assistive technology** (R4). Dimming does that
  without adding a marker to the image. The grid and the viewer have no accessibility properties today, so that part is
  new work.
