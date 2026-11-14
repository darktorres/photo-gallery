# GUI thread plan: the window's catalog work off the Qt thread

Status (2026-11-14): a plan, not started. It has changed no product code.

It has been through fifteen adversarial reviews. The design was reworked around one writer per process after the fifth.
The fifteenth was the first whose reviewers were told nothing of the earlier rounds, and it found faults the briefed
rounds had passed over; the document was then restructured and rewritten, and its list of decisions was folded into the
sections that describe them. This text has not been reviewed since. The reviews' findings are not kept here, so whether
a round before the sixth considered and rejected one writer is not known. What depends on code not yet checked, or on a
measurement not yet made, is under [Open](#open).

Seeded by [NOTIFICATIONS_PLAN.md](NOTIFICATIONS_PLAN.md) (item C2).

## How to read this

| Section | What it holds |
| --- | --- |
| [Why](#why), [Goal](#goal-and-accepted-exceptions) | the measured problems, and what the plan promises |
| [Design](#design) | the parts and how they work: the writer, opening the `Database`, the command thread, the Qt-side values, startup |
| [Rules](#rules) | R1 to R10, which the rest of the document cites |
| [Build order](#build-order) | Parts 1 to 6, in dependency order, and the tests |
| [Shutdown](#shutdown) | closing the window, the system ending the session, and the shutdown's steps |
| [Inventory](#inventory) | every site on the GUI thread today, and where each goes |
| [Measurements](#measurements) | the numbers behind [Why](#why) |
| [Open](#open) | what is not settled, and what was checked against the code |

**Reasons.** Each choice is given with its reason where it is described. There is no separate list of decisions.

**Citations.** Sites are named by symbol, with a `file:line` as it was when read. Line numbers drift: re-read each site
by symbol before acting on it. A path is given in full the first time; a short one after it (`inbox.rs:455`, or a bare
`:136`) is the file last named, and in the [Inventory](#inventory) a path with no crate is under `crates/desktop/src`.
The site lists come from grepping `crates/desktop/src` for `app::get()` and from reading the call graph.

**Words used with one meaning.**

| Word | Meaning | Not to be confused with |
| --- | --- | --- |
| the writer | the catalog's one writing thread | *the write-back*, which writes image files; *the preferences writer*, which writes the preferences file |
| command | an edit the GUI sends to the command thread | the controller's own `Command` enum (`crates/client`), named in full; a *request*, which is a call to one of Part 5's workers; a *model command*, a closure queued to the gallery model that has not yet made a command |
| pending | an edit the user made that is not yet acked or failed. An added folder is pending from the click, before its command is sent | the catalog's `files.pending` (the "pending files" of the Measurements); an entry's `pending_turns`, the turn queued in `file_writes` |
| revision | the catalog's revision of Part 1 | |
| worker | any thread that is not the GUI thread and not the writer | |
| session | always qualified: the `Reader`'s session, the saved session (the listing the window reopens with), or the operating system's session | |
| mark | always qualified: a border mark on a picture, the pending mark of R4, one of the preparation's version marks, or Part 2b's "checkpoint due" | |

## Why

Measured on a copy of the live catalog (see [Measurements](#measurements)):

- **Writes wait on the write lock, up to the 5 s busy timeout** (`schema::SETTINGS`). A rating made during a 7 s hold
  failed after 5.3 s with "database is locked", and the edit was lost.
- **An import holds the write lock for its pairing.** Each 500-file flush held the lock for 2.0 to 2.5 s in release and
  12.4 s in debug. Pairing was 76 to 92% of that hold in release and 98% in debug. A GUI write during an import waits
  for it, and fails if the import holds the lock longer than the timeout.
- **A GUI turn can stall** for 0.51 to 0.84 s when its commit triggers SQLite's automatic checkpoint (the 1000-frame
  default). Once, with the library idle, a turn took 737 ms.

The first two have one cause: the process has several writers, each with its own connection, and they meet on SQLite's
lock with a timeout. Today those are the job (scan, import, borders, thumbs), the write-back, the inbox and the GUI's
pool.

Reads are fast (at most 0.39 ms on a warm connection; 3.0 ms for the first read on a cold pool) and do not wait on a
writer (WAL). Opening a connection can wait: `Catalog::open` retries the shared lock for about 5 s while another process
holds it (`crates/dbfile/src/lock.rs:55-65`), and `Catalogs::with_catalog` opens a connection on the caller when the
pool is empty (`crates/app/src/catalogs.rs:38-43`). The GUI thread must never open one.

## Goal and accepted exceptions

The GUI thread makes no catalog calls and no blocking file or lock calls.

Accepted exceptions, each listed in [Inventory](#inventory) or [Shutdown](#shutdown):

- the shutdown's joins, which run after the window has closed;
- modal dialogs the user is waiting on (the native picker);
- the one-time startup calls that run before the first frame (the log and panic-log setup, `Config::from_env`, the
  pipewire probe, the `gpu_thread` join). Their durations are not measured;
- the clipboard, which Qt requires on the GUI thread;
- a mutex that no thread holds across I/O or a wait (the inbox's `first`, `work` and `listing`, the Yandex bus's);
- the wait on the command thread and the preferences writer when the operating system ends the session;
- env-gated, harness and debug-build work (`PG_RECORD`, the screenshot harness, live reload), which runs only when the
  environment or the build asks for it.

## Design

**Principles.**

- The catalog's connections belong to worker threads, and the window talks to them through messages. The boundary is
  asynchronous.
- A process has one writer. Every write transaction, whoever asks for it, runs on one thread, on the only connection
  that can write. Inside the process no write ever meets another on SQLite's lock, so none fails busy. Order between
  writes is a queue's order, not a race.

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

### Components

By crate. `pg-catalog` may depend only on `dbfile`, `log`, `model`, `platform` and `query` (`tools/crate-layers.txt`),
so it knows nothing of commands, the bus, the job or the write-back.

| Component | Crate | What it is |
| --- | --- | --- |
| `Database` | `pg-catalog` | The one way the app, the command-line tool and the tests open a catalog file. It owns the file's shared hold, starts the writer and the watcher, and hands out read connections and the writer's handle. See [Opening the `Database`](#opening-the-database). |
| The writer | `pg-catalog` | One thread with the one writable connection. See [The writer](#the-writer). |
| Read connections | `pg-catalog` | Every other connection, opened `query_only`. |
| The watcher | `pg-catalog` | One thread with its own read connection, started only when the `Database` is given a function to report to. With the writer it notices another process's writes (R3a). |
| The thread role, `Stop`, the panic marker | `pg-platform` | Three small things `pg-catalog` must be able to use without `pg_jobs`. |
| Command thread | `pg_app` | One thread that owns the GUI's commands: it coalesces them (R6), runs each as an edit-class write, and does what follows a commit. |
| Point lane | `pg_app` | A FIFO executor with its own thread and read connection, opened at startup. It serves details, entry, `file_path`, views and roots. It never drops a request and never waits on the writer (a WAL read). It keeps its page cache. After stop it answers each queued request with an error. |
| Listing lane | `pg_app` | The existing `Reader`, unchanged in behaviour (`crates/app/src/reader.rs:209-238`: the newest request per Reader session and kind replaces the waiting one and cancels the running one). Its connection becomes a read connection. |
| Providers | `pg_app`, desktop | The image providers read through `Images` and the catalog pool on their own threads, as today (`crates/app/src/images.rs`). They do not use the point lane, so its one thread is not a bottleneck for them. |
| Probe threads, the control lane, the pool | `pg_app` | Part 5's workers: see [Part 5](#part-5-the-worker-items). |

**Read connections.** Every connection but the writer's is opened with `PRAGMA query_only = ON`, so SQLite refuses a
write on it. `Catalog.conn` is `pub(crate)`, so nothing outside the crate can switch the pragma off. A read connection
is closed by dropping it, never through `Catalog::close`: `query_only` does not block a checkpoint, and `close` runs
`wal_checkpoint(TRUNCATE)` (`crates/catalog/src/core.rs:52-53`). A worker's read connection belongs to its thread
(`runner.rs:349`, `crates/writeback/src/lib.rs:303`, `crates/app/src/inbox.rs:742`, `reader.rs:152`). The pool's are
shared by the provider threads and are dropped when the pool closes ([Shutdown](#shutdown), step 5).

**One connection the `Database` does not own.** The thumbnail cache opens its own SQLite file and attaches the catalog
to it, on a `query_only` connection that it opens and drops for each call (`crates/thumbcache/src/reader.rs:24-28`,
`:62`, called from `crates/thumbs/src/lib.rs:180`). `thumbcache` and `catalog` do not depend on each other, so the
`Database` cannot perform that attach. It stays as it is, and is named here as the exception. It only reads.

**Two rules for every write.**

- *A transaction holds only SQL.* Anything computed inside it keeps every other write waiting, so whatever a write needs
  computed is computed by its caller first. Part 2a is the first case: the import's pairing.
- *A write decides inside its transaction.* A worker reads on one connection and writes on another, so what it read may
  be stale by the time its write runs. A closure re-reads, inside the transaction, whatever its write depends on, as
  `store_analysis` does with `IS_CURRENT`. The transaction type has only writers today (`core.rs:209-237`); it gains the
  reads those checks need. Three writes pre-read on the connection today so as not to take the lock for nothing
  (`clear_failures`, `crates/catalog/src/ledger.rs:117-120`; `replace_notices`, `inbox.rs:97-113`; `set_mark`,
  `lifecycle.rs:102`). Each keeps its pre-read, on the caller's read connection, and reads again inside the transaction.

### The writer

One thread with the one writable connection, held for the life of the process. It takes the keeper's place: the keeper
is only the prepared connection held open (`crates/app/src/app.rs:62`).

**Its one entry** is `write(class, stop, |tx| ...)`. The caller blocks until the transaction commits, and gets the
closure's result and the revision. The GUI thread never calls it (R8).

- Every transaction begins `IMMEDIATE`, as `write_transaction` does today (`crates/catalog/src/transaction.rs:13-16`).
- Nothing is interrupted. A new edit waits for what is in progress (one transaction or one checkpoint) and for the edits
  ahead of it, not for a queue of job writes. How long one transaction can be is in Part 1.
- A write the writer has taken is tried until it has the lock. An edit that arrives meanwhile is next.

**Classes**, served in this order, first in first out inside each:

| Class | Who | Why this place |
| --- | --- | --- |
| edits | the command thread | what the user is watching |
| service | the write-back's and the inbox's writes | small and few |
| job | the job's passes, the preparation's marks, the command-line tool's writes, the fixtures' | what can wait |

Edits come at a person's pace and service writes are small, so the job is not starved.

**The result.** `write` returns `Result<(T, Revision), WriteError<E>>`, where `E` is the closure's own error type:

| `WriteError` | Meaning |
| --- | --- |
| `Stopped` | the call ended while waiting; nothing was written |
| `Failed(E)` | the closure returned its error; the transaction was rolled back |
| `Catalog(_)` | beginning or committing failed |

So `Stopped` is a variant of its own and is never looked for inside an error chain: a walk of the chain would miss it
wherever an error is flattened (Part 1). The passes' closures return `anyhow` errors today
(`crates/import/src/lib.rs:54`, `crates/borders/src/lib.rs:68`, the scan's `write_batch`), the inbox's a
`pg_catalog::Result` (`crates/app/src/inbox.rs:541-553`); each keeps its own.

**Stops.** Every call carries its caller's stop: a `Stop` from `pg-platform`, a predicate that is polled. `pg-platform`
has no such type today.

- *A stop ends a call only while the call is waiting*: queued behind others, or at the head while the writer waits on
  another process's lock. Such a call returns `Stopped`. A call that finds the writer free runs, and a closure that has
  begun runs to its commit: it is never cut, which would leave its transaction half done. After the `Database` has
  closed, every call returns `Stopped`.
- *Withdrawing a waiting call.* The caller and the writer meet on the queue's lock: either the writer has taken the
  call, and the caller waits for it to finish, or the caller removes it. So a borrowed closure is never left with the
  writer after its caller has returned.
- *A waiting caller polls.* Whoever raises a stop knows only its own condvar (`Control::request_stop`,
  `crates/jobs/src/control.rs:117-121`; the service's quit, `crates/jobs/src/service.rs:111-116`; the inbox's close,
  `crates/app/src/inbox.rs:455-456`), not the writer's queue. So a waiting caller waits in slices of 50 ms and checks
  its stop between them, as `checkpoint_within` does (`control.rs:192-208`). The busy handler checks the current call's
  stop the same way; it reaches it through a slot, since the handler itself is `Send + 'static`.

| Caller | Its stop | Grace | Notes |
| --- | --- | --- | --- |
| command thread | its own stop | none | By then its queue is empty, or the quit did not wait for it ([Shutdown](#shutdown), step 1). In Parts 1 to 3 each command also carries a 5 s deadline (Part 1). |
| the job's passes | a new quit flag on `Control` | 4 s | Below. |
| the write-back | its `Service`'s quit | 4 s | So a `write_done` after a finished copy is not dropped. Its `process`, `finish_copy` and `keep_turn_in_catalog` do not receive the `Waker` today (`crates/writeback/src/lib.rs:411`, `:579`, `:649`, `:697`); the stop is passed to them. |
| the inbox | its close | 5 s | The drain (`crates/app/src/inbox.rs:79`), which `report_lost` (`inbox.rs:901`) uses too. Its stop is a `bool` under a mutex today, with the deadline set lazily (`inbox.rs:455`); it becomes the same flag type. |
| `Database::open` | the one its caller passes | none | Part 1, R9. |
| the command-line tool, the tests | never | | |
| no call current (the close's `TRUNCATE`) | | gives up after 5 s | Today's timeout. |

- *Why a grace.* A stop that ended a write at once would drop what today gets 5 s to commit: the job's final flush and
  the lease's release, whenever the writer is briefly busy. A lease left unreleased makes a restart within 60 s wait on
  its own lease. The grace is 4 s, shorter than the 5 s the runner waits for the job (`JOB_STOP_WAIT`), so a write that
  gives up returns before the runner does, and the job is not reported abandoned for it.
- *The job's stop is a new quit flag, not `Control::stop`.* `Control::stop` is the job's preemption flag: every new
  request raises it (`crates/library/src/runner.rs`, `request_stop`), a round clears it, and its contract is "it commits
  what it has" (`crates/jobs/src/control.rs:116`). The passes write after it is raised: the final flush
  (`crates/jobs/src/pipeline.rs:211`, `:266-268`), the scan's stop batch (`crates/scan/src/lib.rs:580-582`), the thumbs
  ledger (`crates/thumbs/src/lib.rs:338-339`), the pairing, `mark_missing` and `finish_scan`
  (`scan/src/lib.rs:536-540`), and the lease's release. Used as the write stop it would drop those. `Control` gains a
  quit that only `Runner::stop` sets, and the job's writes carry that.
- *Where the raise time lives.* The job's quit and the write-back's are two flags. The passes receive only `&Control`;
  the write-back reaches its `Service`'s quit through the `Waker` (`crates/writeback/src/lib.rs:359`), and that quit
  keeps only a flag today (`crates/jobs/src/service.rs:111-116`). Both become one `pg-platform` type, a flag that
  records when it was raised, which `Control` and `Service` each hold, so a `Stop` can read it without `pg_jobs`.

**The thread role** (`pg-platform`). A thread-local with three values: GUI, writer, worker (the default). The writer
sets its own at start. The Qt main thread is GUI from when the event loop is about to start until it returns; before and
after, it is a worker, so the open (until Part 3) and the shutdown's joins may block on it. `write` checks the role: it
refuses the GUI thread, and it refuses the writer's own thread, where a closure that called `write` would wait on
itself.

**The closure may borrow, but not a read connection.** The caller blocks until the closure has run or been withdrawn, so
the hand-off is scoped, like `std::thread::scope`. The flushes borrow pass-local state today
(`crates/import/src/lib.rs:50-89`, `crates/borders/src/lib.rs:60-117`, `crates/catalog/src/pair.rs:96-229`), and the
alternative is to restructure every flush to own its data. The lifetimes of the closure and of the stop are erased in
one documented `unsafe` block in `pg-catalog`. `write` requires `F: Send` and `R: Send`. That alone does not keep the
caller's read connection out: the connection is `Send`, and the passes hold it as `&mut Catalog` today
(`import/src/lib.rs:32`, `borders/src/lib.rs:50`, `scan/src/lib.rs:379`). So the passes hold their read connection as
`&Catalog`. A shared reference to it is not `Send`, because the connection is not `Sync`, and a closure that captured it
does not compile. Without that, a closure could read stale state through it on the writer's thread.

**A panic in a closure.** The transaction's drop rolls it back, and the writer keeps its connection: reopening it would
add a failure path for nothing. The panic is resumed on the caller's thread, where the caller's own handling applies:
the job catches a pass's panic, and the write-back catches a panic inside a job as that job's failure (`panics::guard`,
`crates/writeback/src/lib.rs:411`).

- *The marker.* The panic hook asks a thread-local whether the panic will be caught (`CAUGHT`,
  `crates/jobs/src/panics.rs:11-14`), and shows the fatal box when it is not set
  (`crates/desktop/src/panic_log.rs:21-33`). The hook runs on the thread that panics, which is the writer's, and
  resuming the panic on the caller does not run it again. So each `write` call carries its caller's marker value
  (`caught()`, `panics.rs:28`), and the writer sets its own marker to that value around the closure.
- *Who then gets the box.* A caller that catches gets none. A caller that does not still gets it, as today: the inbox
  thread, the write-back outside its job guard, and until Part 3 `Database::open`'s caller. From Part 3 the startup
  worker catches (R9), so the hook shows no box, and the failed-open path shows the one box.
- *The writer is paused while the box is up.* The box is shown on the thread that panicked (`panic_log.rs:25-32`), so
  every write waits until it is dismissed, and the log line names the writer's thread. The process does not end there:
  the hook returns after the box (`panic_log.rs:19-34`), the caller's thread dies with the resumed panic, and the writer
  goes on. That is accepted.
- *The move.* The marker and what reads and sets it, `catch` and `caught()` (`panics.rs:20-28`), move to `pg-platform`,
  with one new setter for the writer. `catch` uses only the standard library; `guard`, `message` and the rest stay in
  `pg_jobs` and call it. `pg_jobs` may not re-export it (`tools/boundaries.py`), so its six outside callers are
  repointed in Part 1: two in `crates/app/src/yandex.rs`, one each in `yandex/images.rs`, `reader.rs` and
  `crates/library/src/runner.rs`, and the hook's reader (`pg_app::reexport::panic_caught`,
  `crates/desktop/src/panic_log.rs:21`), whose line in `reexport.rs` goes.

**The statement cache.** The writer's one connection now prepares every role's cached write statements, about 30 across
the ledger, the pairing, the inbox, the roots and the queue, against rusqlite's default of 16
(`crates/catalog/src/writers.rs:24-26`). Its capacity is raised.

### Opening the `Database`

`Database::open` does what `Catalog::prepare` does today (`crates/catalog/src/core.rs:69-125`), in this order. Steps 1
to 3 and 5 to 7 run on the opening thread.

1. Take the preparer's lock (`core.rs:69`, `lifecycle.rs:43-47`).
2. Rebuild or migrate, and make the checks (`core.rs:69-114`). The migration step begins a write transaction on every
   open, even with nothing to migrate (`crates/catalog/src/schema.rs:410-414`). It is the one transaction that is not
   the writer's.
3. Register the SQL functions on the connection (`core.rs:120`), raise its statement cache, and install the busy handler
   (after `SETTINGS`, which would replace it).
4. Start the writer's thread and move the prepared connection to it.
5. Make the preparation's marks (`core.rs:115-119`) through the writer, as job-class `write` calls with the open's stop.
6. Drop the exclusive hold if it has one and take the shared one, with the preparer's lock still held
   (`core.rs:123-124`). The `Database` owns the shared hold, not the connection.
7. Start the watcher, when a function to report to was given.

`Catalog::open` and `prepare` stop being public. The `Database` is a value the app owns and passes, not a `static`.
Everything that takes the catalog's path today takes the `Database`: `Runner::start` (`crates/library/src/runner.rs`),
the write-back's `Writer::start` (`crates/writeback/src/lib.rs:222`, restarted in `tend`), `pg_thumbs::run`
(`crates/thumbs/src/lib.rs:173`) and `Inbox::start` (`crates/app/src/app.rs:66`). It also gives what callers read from
the prepared catalog today: whether it was rebuilt (`app.rs:64`, `crates/cli/src/report.rs:62`), and the file's path,
which the thumbnail cache attaches by (`thumbs/src/lib.rs:180`).

**Why this order.** When it is the sole opener, `prepare` holds the file exclusively until after its marks
(`core.rs:92`, `:115-123`), and every connection takes its own shared hold when it opens (`core.rs:136`), which would
wait 5 s on that exclusive hold and fail. So the prepared connection itself becomes the writer's, and the watcher's and
every read connection open only after step 6. With another process already open there is no exclusive hold
(`core.rs:91-95`); the marks run under the preparer's lock alone.

**The waits at open, and their stop.** Three things can wait behind another process:

| Wait | Mechanism | Ends on |
| --- | --- | --- |
| the preparer's lock (step 1) | the file-lock layer's loop of 100 tries 50 ms apart (`crates/dbfile/src/lock.rs:12-13`, `:29-37`) | the open's stop, which `Lock::take` takes for the catalog |
| the migration step's `BEGIN` (step 2) | SQLite's busy handler | the open's stop, which the handler's slot holds from step 3 until the open returns |
| the marks (step 5) | the writer's queue and busy handler | the open's stop, which each `write` carries |

- Only the preparer's lock can wait in the file-lock layer: the shared hold is taken while that lock is held, and the
  only exclusive taker is `prepare` itself (`core.rs:92`), which does not wait. A preparer holds its lock for as long as
  its own marks wait, which for the command-line tool has no limit; the busy state (R5) is what tells the user.
- All three report to the busy function (R5), so the placeholder shows the busy state for any of them.
- *Read connections opened later do not wait.* The `Database` keeps its shared hold from step 6, so no other process can
  take the exclusive hold while it lives. A read connection opened by the pool on a provider thread therefore cannot sit
  in the file-lock loop. The thumbnail cache takes the same kinds of lock on its own file
  (`crates/thumbcache/src/schema.rs:245`, `:371`); it keeps today's bounded form, since a cache that fails to open is
  only disabled.
- *If a mark returns `Stopped`*, the open ends the writer's thread it started and returns `Stopped`.

### The command thread

One thread in `pg_app` that owns the GUI's commands.

- It coalesces the commands waiting in its own queue (R6) and runs each as a `write(edits, ...)`.
- Then, on its own thread, it does what follows a commit: it replies with the ack, wakes the write-back (`wake_writer`,
  `crates/app/src/library.rs:176-180`), queues an added folder's scan (`Runner::scan`), or tells the job a folder is
  gone (`Runner::folder_removed`). None of that runs on the writer: these take locks, and on the writer's thread they
  could deadlock the shutdown, where the main thread holds the write-back's slot mutex while the write-back waits on the
  writer.
- A panic in a command is caught there: that command's ack is a failure, and the thread goes on.
- `Command` and `Ack` carry `pg-model` types only (R10).
- *Acks have one receiver.* Commands start in three objects: the gallery model (`gallery/edits.rs`), the `Shell`
  (`shell.rs:312-331`, `:496`) and the `Library` (`desktop/src/library.rs:211-237`). Every ack is posted to the Qt
  thread and handled in one place, beside the controller's core: it updates the pending set, hands a file's ack to the
  gallery model, a view's to the `Shell` and a folder's to the `Library`, and sends the controller its event when a
  failure takes a close back ([Shutdown](#shutdown)). The objects register with that place when they are constructed, so
  a window rebuilt by a live reload receives the acks of commands the old one sent. The pending set and this routing are
  thread-locals of the Qt thread, like the core (`crates/desktop/src/controller.rs:30-33`).

### Qt-side values

- **Stored.** Each value the GUI holds from the catalog stays where it lives today, and gains the revision it was read
  at: the grid's listing and the listing held under the viewer (`pg_client::Grid`, `crates/client/src/grid.rs:109`,
  `:121`), the details slots, the folder tree, the facets, the saved views and the roots. R2 says how they change. A
  stored value is what one read or one ack returned: there is no second cache of the catalog with its own invalidation.
- **The details slots.** Details are point-lane values, not derived from the grid. The GUI keeps four slots: the
  viewer's open item, its previous and its next (so arrow navigation finds the neighbours ready), and the grid's
  selection, whose line `refresh_derived` renders (`gallery/model.rs:264-272`). Each slot holds one result and its
  revision. They replace the two-entry details memo (`gallery/state.rs:56`, used at `:129-130`).
- **The saved views** are held whole in one place on the Qt thread, beside the core. `Shell` holds only their names
  today (`shell.rs:48`, `:117-129`). The startup worker's slot gives the first value (R9); each view command's ack and
  each refresh replace it.
- **Pending.** One set on the Qt thread holds every edit made and not yet acked or failed, whatever it names: a file, a
  view, the session, a folder. That includes an add whose command is not yet sent (R7). The grid is given the file
  commands to draw over its entries. The cap reads this set (R5). The close's decision and the screenshot harness read
  it together with the count of model commands posted and not yet run ([Shutdown](#shutdown)).
- **What shows** is the stored value with the pending commands applied on top, with two exceptions that R4 gives: the
  image key and aspect, and the roots list, show the stored value only. Reads and acks change only the stored layer;
  commands and acks change only the pending set.
- **They are plain types.** The pending set, the 200 ms mark (on virtual time, like `Deadlines`) and the close's
  decision live in `pg_client`, or as free structs in the desktop crate, not inside a QObject. The desktop's tests
  construct no QObject today (`shell.rs:572-619` drives a bare `Controller` and free functions), so that is the only
  place their tests can live.

### Startup

`app::open` runs on a startup worker, which then reads the start state. The window shows a loading placeholder until
both are done. This is a cost the goal forces, not a fix for a visible flash: once loaded, the first content frame shows
the start fields as today.

**The start state is read on the worker, not queried by the GUI.** `ShellRust::default` returns the listing, the panels
and the size synchronously (`crates/desktop/src/shell.rs:39-56`), from `Core::new`, which needs the session's listing,
the applied view and the preferences before it builds the controller (`crates/desktop/src/controller.rs:38-66`).
`Filters`, `FacetChoices` and `FolderModel` reach the same core (`filters.rs:32`, `facet_choices.rs:54`,
`folders.rs:236`). There is no "loading" controller, and the plan adds none. The worker reads the saved session's
listing, the applied view, the saved views, the roots, the library's first status and the preferences, leaves them in a
slot beside the app, posts Ready and ends. `Core::new`, the views' holder and the `Library` read the slot; they do not
empty it. The roots are in it so the window does not say "no folders" until a point-lane read lands (`Main.qml:562`,
`GalleryGrid.qml:94-99`). A failed read gives the default state and a notice, as now.

**The app stays a process-wide value.** The bridge objects are QML elements that QML constructs through `Default`
(`#[qml_element]` in every `*_bridge.rs`), so they cannot be handed the app when they are made. `app::get()` stays, as
`APP.get().expect("app::open not called")` (`crates/desktop/src/app.rs:44-51`). So no bridge object that reaches the app
may exist before Ready. Today `app::open` runs on the main thread before the GUI application exists
(`crates/desktop/src/lib.rs:164`). During QML load the app is reached by `Shell` (in `Default`), `Settings` (on first
property read, `settings.rs:22`, `:26`), `Notices` (`start`, `notices.rs:254`), `FolderModel` (`reload`,
`folders.rs:180`), and `Filters` and the six `FacetChoices` (through the core). `Library` reaches it later, from
`startHooks`.

**One engine load, with a `Loader`.** The root window creates one bridge object, `Startup`, which reaches nothing. A
`Loader` creates the main content when `Startup` reports Ready. The app and the slot are set before Ready is posted, so
the `expect` cannot fail by construction. A `Startup` constructed after that, by a live reload
(`cpp/live_reload.cpp:100`), finds the slot set and reports Ready at once. The controller's core is built once per
process, so a reload builds no new core.

**All of `Main.qml`'s content moves into the loaded component.** What the root must keep or forward: the title (bound to
`shell.folder`, `Main.qml:21`); `current` (an alias into the model, `:47`, which cannot reach into a `Loader`); `ready`
and `shellCore`, which `cpp/screenshot.cpp:82`, `:177` read (`:74`, `:116`); `onClosing`, `onActiveChanged` and
`onDevicePixelRatioChanged` (`:138`, `:369`, `:734`); and `Component.onCompleted` (`:736-748`). A `Loader` parents its
item, so the harness's `findChildren` still finds the objects. The trace stages change meaning: `firstFrame`
(`:117-124`) moves to the loaded content, or it would time the placeholder, and `init_done`, `qml_load` and `qml_loaded`
(`lib.rs:167`, `:188-190`) are renamed for what they now time.

**Waiting behind another process.** The open's waits carry the worker's stop ([Opening the
`Database`](#opening-the-database)). After 10 s the placeholder shows the busy state (R5).

**How the open can end.**

| The worker reports | The GUI thread |
| --- | --- |
| Ready | loads the main content |
| a failed open | shows the native error box, then leaves through `QCoreApplication::exit(1)` |
| a stopped open | shows no box and runs the ordinary shutdown |
| anything, after a close before Ready | loads nothing, shows no box, and runs the ordinary shutdown |

- *A failed open.* Today it is fatal before Qt exists: a native error box, then `process::exit(1)`
  (`crates/desktop/src/lib.rs:164-166`, `alert.rs`). There is no app and so no inbox for a notice. It uses
  `QCoreApplication::exit(1)` now, since Qt and its render thread are running by then. A panic on the worker is caught
  there and reported as a failed open: uncaught, it would end only the worker, and the placeholder would stay up for
  good.
- *A stopped open* means the user asked to close before Ready.
- *A close before Ready.* There is no `Shell` and no controller yet, so the close's decision of [Shutdown](#shutdown)
  does not apply. `Startup` holds a closing state of its own: the root's `onClosing` asks it, with no flush and no
  controller event. The window stays, saying it is closing, and the worker's stop is raised. A wait ends at once; a
  migration or a transaction in progress runs to its end and cannot be cut short, so the open may still succeed.
  `app::stop` and `app::shutdown` already tolerate a missing app (`crates/desktop/src/app.rs:55`, `:64`). The window
  does not vanish while the process still holds the file: the next launch would then fail to open it.

## Rules

- **R1, a read never waits and never hides a pending edit.** A read result goes to the stored layer, under R2. The
  pending set is drawn over it, so a file with a pending command keeps showing its pending value (R4). No result is
  refused or merged because a command is pending; R2 refuses a result only for being older than what is held. A read
  that waited for a pending write would tie the point lane to the writer, and a point lane that replaced a waiting
  request, as the `Reader` does, would drop results; it does neither ([Components](#components)).
- **R2, every stored value carries its revision and only moves forward; a change that a result missed is read again.**

  | Stored value | Read by | Applies when | Read again on |
  | --- | --- | --- | --- |
  | the grid's listing; the listing held under the viewer | listing lane | its revision is at least the held listing's | `Scanned`, `Imported`, a refresh of everything |
  | one entry of a listing | an ack, or the point lane | its revision is at least that entry's own (the sparse map) or, without one, the listing's | a file change above its revision; a note |
  | a details slot | point lane | its revision is at least the slot's | a file change above its revision; a note; a listing applied above its revision; the slot's file changing |
  | the folder tree, the facets | listing lane | its generation is the newest asked for (`folders.rs:174-199`, `filters.rs:154-183`) | `Scanned` (tree), a filter change (facets), a refresh of everything |
  | the saved views, the roots | an ack, or the point lane | its revision is at least the held one | a refresh of everything |

  - *Forward only.* A result or an ack applies only if its revision is at least the held one. A refused result is always
    older than what is held. So a roots read made before a folder's removal cannot land over the removal's ack.
  - *The sparse map.* An entry replaced since its listing was read (by an ack or a point read) carries its own revision
    in a sparse map beside the listing. When a new listing is applied, an entry whose own revision is above the
    listing's keeps its value at the row's new position, and stays in the map; the entries at or below the listing's
    revision leave it. The map is never simply emptied: an older listing would then let an older point read land over a
    newer ack.
  - *A file change.* Three things name one file and a revision: an ack, `Turned`, and `Rewritten` (Part 3). If the
    revision is above the held one for that file's entry or for a details slot holding that file, the GUI asks the point
    lane for what the event did not itself carry: after an ack, which carries the entry, the details only; after
    `Turned` or `Rewritten`, the entry and the details. Nothing else is read: the facets, the tree and the listing are
    not re-read for one file's change (R4). One at or below the held revision asks for nothing.
  - *In flight.* A read is in flight for a kind of value from when its newest request is sent until that request's
    result arrives, whether it is then applied, refused or an error. A request that was replaced or cancelled is not in
    flight; its replacement is. One exception: a listing that arrives while the viewer is open is held
    (`crates/client/src/grid.rs:285-288`) and applied at release (`gallery/model.rs:122-126`); it stays in flight until
    it is applied or replaced.
  - *Notes.* While any read is in flight, the GUI notes each file change: one note per file, with the newest revision,
    so the notes are bounded by the files that changed. When a result is applied, every file in it whose noted revision
    is above the result's is asked for again through the point lane: its entry, and its details if a slot holds it. The
    notes are dropped when nothing is in flight. This generalizes what the grid does today for turns
    (`note_turns_written`, `take_turned` and `refresh_turned`, `crates/client/src/grid.rs:345-354`,
    `desktop/src/gallery/model.rs:158-160`), and it covers what a revision check alone loses: a file changes while a
    listing that contains it is loading, and the older snapshot then lands.
  - *A change to everything is not a note.* `Foreign` and a lag are the only causes that name no file. They are kept as
    causes of the spaced refresh (R3): a listing that lands while one is kept is not asked for again at once.
  - *An applied listing asks for the details again.* Today applying a listing forgets the details (`forget_details`,
    `gallery/model.rs:146`), so every reload re-reads them. Otherwise the info line would mix the new entry with old
    details after an import (`gallery/items.rs:184-193`).
  - *A failed read* ends its flight and changes no revision. What it stores is what today's code shows for the failure:
    a failed listing the empty list (`gallery/model.rs:104-108`); a failed tree the empty tree, which still emits
    `loaded`, because the start gate waits for it (`folders.rs:180-193`, `crates/client/src/controller/start.rs`);
    failed facets the default facets (`filters.rs:162-175`). A failed entry, details, views or roots read changes
    nothing.
  - A result's revision is the one its own read transaction saw (Part 3), so it bounds the data the result holds.
- **R3, lost events.** The bus drops the oldest event for a lagging subscriber and counts the drops
  (`crates/jobs/src/events.rs:82-85`). The library pump reads that count (Part 3). When it is above zero, the GUI
  refreshes everything (below). The results apply under R2. The inbox's `Changed` events carry the whole listing, and
  its pump keeps only the newest (`desktop/src/notices.rs:261-265`), so a lost one costs nothing. A lost `Toast` is
  dropped silently (the inbox bus holds 256 events, `crates/app/src/inbox.rs:303`); the pump logs the dropped count. The
  window shows only the last toast of a pass anyway (`desktop/src/notices.rs:294`).
  - *"Refresh everything"* is one list, used by R3 and R3a: the grid's listing and the folder tree (through the listing
    lane, as `Scanned` does today, `desktop/qml/Main.qml:127-128`, `:255-265`), the facets (`Main.qml:142-145`), the
    details slots, the views, the roots, the status lines, and the inbox's poke (`desktop/src/library.rs:134-160`). The
    grid alone is not enough: the tree, the facets and the status lines would stay stale. The refresh re-lists, so it
    drops a file rated out of the filter, as any reload does today.
  - *It is spaced.* A full listing of the large root takes up to 430 ms (`crates/app/src/reader.rs:178-183`) and a full
    layout pass on the GUI thread. A refresh is not started while the last one is in flight, nor within 5 s of the last
    one's start; a cause that arrives meanwhile is kept, and one refresh follows. The 5 s is chosen, not measured.
- **R3a, writes from another process.** The command-line tool writes the same catalog through its own `Database` and its
  own writer: scan, import, remove, the marks that make files pending import, borders, thumbs and the write-back
  (`crates/cli/src/lib.rs:218`, `:228`, `:248`, `:279`, `:324`). The screenshot fixtures
  (`tools/shots/fixture.py:82-102`) write rows directly with no revision; they run with no window open.
  - *The tool keeps writing the catalog itself.* The alternative is one process owning the file, with the tool sending
    its work to a running window. That would remove this rule and every wait on another process, at the cost of a
    protocol between the two. Two processes on one WAL database is a supported arrangement, so it is not taken.
  - *`data_version` alone is not the test.* It moves whenever any other connection commits
    (`crates/app/src/inbox.rs:716`), and that includes this process's writer.
  - *The test is a revision this process did not write.* The `Database` keeps the last revision it has seen, raised only
    upward. It starts as the stored revision, read in the writer's first transaction, so nothing is reported at open.
    Two places compare the stored revision with it:
    - the writer, at the start of every transaction. If the stored revision is not the last seen, another process
      committed in between. The writer raises the last seen to it and reports it at once, before its own work: the
      report only publishes on a bus, and a long transaction must not delay it. This catches a foreign write that an own
      commit follows before the watcher looks;
    - the watcher, which reads `data_version` once a second (it costs microseconds) and, when it has moved, reads the
      stored revision. If that is above the last seen, it raises the last seen to it and reports it. This catches a
      foreign write while the writer is idle.

    Each of these two raises is an atomic maximum, and of the two only the side whose raise changed the value reports.
    So one foreign revision is reported once, whichever sees it first. The writer's raise after its own commit (below)
    never reports: that revision is this process's own.
  - *The watcher must not see an own commit as foreign.* The writer raises the last seen revision after its commit, so
    the watcher could read the new stored revision in between. The writer therefore counts its commits in a sequence
    counter, raised before `COMMIT` and again after the last seen revision is raised, also when the commit fails. The
    watcher reads the counter, then the stored revision, then the last seen revision, then the counter again, and tries
    again when the two counts differ or a commit is under way.
  - *What is built when.* The comparison, the watcher's thread and the commit counter are built in Part 1, with the
    `Database`. Until Part 3 the app gives the `Database` no function, so no watcher starts and the writer reports to
    nobody; the last seen revision is kept from the start, so Part 3 can add the route without a false first report.
  - *The route.* The `Database` calls a function the app gives it. The app publishes `Change::Foreign { revision }` on
    the change bus, and the library pump answers it by refreshing everything, spaced as R3 says. A command-line pass
    commits every second or two (an import's flushes, a thumbs pass's ledger, a scan's batches), so without the spacing
    it would refresh the window about once a second for its whole run. The app creates the bus in `App::open`; today
    `Library::start` creates it (`crates/app/src/library.rs:99`).
  - *The inbox is not changed by this.* Its own `data_version` comparison (`inbox.rs:803-804`) also tells it of this
    process's ledger writes (`inbox.rs:8`), and it keeps it.
  - *How soon.* Another process's write reaches the window within about a second when no refresh started in the last 5
    s, and within 5 s otherwise. The same spacing applies to a lag: a `Turned` the bus dropped is made good by a
    refresh, so those files can show their old picture for up to 5 s, where today's re-read is immediate.
- **R4, a pending edit is shown and marked.** Its value comes from the pending set; the stored layer is changed only by
  reads and acks.
  - *A pending value is marked.* It is dimmed, which needs no marker on the image, and assistive technology is told it
    is pending. The mark appears only after the command has been pending for 200 ms: an ack normally lands quickly, and
    a mark shown at once would flicker on every edit. So for up to 200 ms a new rating shows unmarked. The 200 ms is
    chosen; how long an ack takes is not measured ([Open](#open)). One timer serves every pending command; no row has
    its own.
  - *A pending rating or border mark* shows its new value at once.
  - *A pending turn does not turn the picture.* The image key is `<id>@<mtime>.<rotation>.<pending_turns>`
    (`desktop/src/gallery/items.rs:41-46`), and on a thumbnail miss, the placeholder and the full image the providers
    turn by what the catalog holds (`crates/app/src/images.rs:227`, `:272`, `:290`). A key that ran ahead of the catalog
    would make Qt cache unturned pixels under the turned key. So the key and the layout's aspect (`Entry::aspect`,
    `crates/model/src/grid.rs:75`) come from the stored layer only. A pending turn shows as the pending mark alone.
  - *A turn has two later moments.* At the ack, the stored entry holds the queued turn (`rotation` is `files.rotation`
    and `pending_turns` is the queued `file_writes` value, `crates/catalog/src/browse/read/grid.rs:26-27`, `:68-69`),
    the key changes, and every view turns, as today after `queue_rotation` commits
    (`desktop/src/gallery/edits.rs:27-47`). At `Turned`, the write-back has rewritten the file, and the entry is read
    again (R2).
  - *The ack* carries what the catalog now holds, read inside the command's own transaction, so it is one snapshot at
    the ack's revision. It goes to the stored layer under R2, and the command leaves the pending set. A command that
    changed nothing is acked with the revision it read.

    | Command | Its ack carries |
    | --- | --- |
    | a file command (rate, turn, border mark) | the file's entry. For a file in no listing the GUI holds it has nowhere to apply; the command just leaves the pending set |
    | saving or deleting a view | the saved views, whole (`views().all()` already returns them, `crates/app/src/views.rs:46-63`) |
    | the session's save | nothing |
    | adding a folder | the roots, the root's normalized path and whether it was added, as `add_folder` returns today (`crates/app/src/library.rs:298`, used at `desktop/src/library.rs:212-223`) |
    | removing a folder | the roots and the removed root's path |

  - *A command that fails* leaves the pending set, and the stored value shows again. No read is needed: the stored layer
    never held the failed value. The user is told as today, by the edit's failure notice (`edit_failed`,
    `desktop/src/gallery/edits.rs:27-30`).
  - *Whatever reads an edited field reads the shown value*, not the stored entry. One accessor gives the shown rating
    and mark and the stored value for everything else. Its readers:
    - the row's roles, which most need the pending value: `rating()` and `border_marked()` (`gallery/items.rs:109`,
      `:130`, reached from `gallery/model.rs:54`) become that accessor, which covers the rows, the viewer and the
      selection line;
    - the border toggle, which reads the mark to decide what to send (`shell.rs:486-488`, `gallery/edits.rs:98`): from
      the stored entry, a second toggle while the first is pending would send "mark" again;
    - the crop's ids (`edits.rs:120-121`), the viewed item's rating and mark, the selection line's symbol
      (`gallery/model.rs:266-272`, `:297`, `:324`), and the info line (`gallery/items.rs:185-191`).

    Today they do not share one: the toggle reads `self.grid.item(at)`, the crop reads `self.grid.entries()`, and
    `entry(item)` (`gallery/state.rs:97`) returns the stored `&Entry`. The image key, the aspect and the layout's `plan`
    keep reading the stored entry.
  - *Applying a view* stays an in-memory lookup in the shown views (a pending save carries its view, so a view applied
    before its save is acked is found): `shell.applyView` is called synchronously from QML (`Main.qml:275-285`,
    `shell.rs:375-381`).
  - *A rating does not re-list.* A file rated out of the current filter stays until the next reload, and the facets stay
    as they were (`crates/client/src/grid.rs:393-395`, `Main.qml:260-262`). That is today's behaviour, kept.
  - *The roots list stays the stored one.* The library panel lists `library.roots` (`LibraryPanel.qml:46-57`), and a
    removal maps its row to `root_ids[row]` (`desktop/src/library.rs:96`, `:227`). If pending changes altered that list,
    a removal could hit the wrong folder. So `roots` and `root_ids` are the stored list only.
    - A pending removal dims its row and disables its button. A pending add shows as a separate line below the list,
      which no row index reaches.
    - With the first add pending, the empty states say so: the grid's and the panel's "no folders" texts
      (`Main.qml:562`, `GalleryGrid.qml:97`, `LibraryPanel.qml:60-64`) show the pending add in their place.
    - On a removal's ack the `Library` object updates the roots and then emits `scanned()`, as it does today on success
      (`desktop/src/library.rs:231-234`), which reloads the tree and the grid.
    - A folder that was already a root is reported as today (`FolderAlreadyAdded`, `desktop/src/library.rs:215-217`):
      the ack carries the flag, and the ack's handler posts the notice.
- **R5, the lock is busy only when another process holds it.** Inside the process the writer is alone, so a busy write
  means the command-line tool holds the lock.
  - *The wait.* The writer's connection has a busy handler in place of the 5 s timeout (rusqlite's `busy_handler` takes
    a closure, `src/busy.rs:55-57`). It sleeps and tries again, and gives up when the current call's stop is raised. It
    is installed after `SETTINGS`, because `PRAGMA busy_timeout` replaces it (`crates/catalog/src/schema.rs:377-378`; so
    does `Catalog::set_busy_timeout`, `core.rs:190`). No write is lost to the busy timeout, and no caller has a busy
    loop of its own; a write ends unwritten only when its own stop is raised.
  - *The busy state.* A function is told when the process has waited 10 s on another process, for a write of any class
    or for one of the open's waits, and again when that wait ends: the lock is had, or no write is left waiting for it.
    The 10 s counts the unbroken wait, not one write's: when a write is stopped and the next takes over the same wait,
    the state does not clear and start again. The 10 s is chosen.
  - *Where it goes.* The desktop passes the function into `app::open`, which passes it to `Database::open`: the
    placeholder needs it before the app exists. It runs on the thread that is waiting (the writer's, or the opening
    thread) and posts to the Qt thread, to whichever object is current: `Startup` before Ready, the `Shell` after. One
    process-wide holder, under the same lock as the start-state slot, names that object and keeps the current busy
    value, so Ready and the busy state cannot race `Startup`'s construction, and an object that registers later (the
    `Shell` at Ready, or after a live reload) starts with the right value.
  - *What shows.* A QML property set on the Qt thread, not a database notice, since a notice is a write the same lock
    would hold. A transient toast says so. `Library.busy` already exists (`cpp/screenshot.cpp:105` reads it), so the
    property has another name. It is built in Part 4; in Parts 1 to 3 the function only logs.
  - *The cap is the GUI's.* The pending set holds at most 10,000 file commands. A file command beyond that is refused on
    the Qt thread, before it is sent: the GUI is the only place that knows the count when a command is made. Commands
    that name no file are few (about one session save a second at most) and are not counted, so the cap never refuses
    the close's own session save. The value is chosen, not measured.
  - A command that fails for another reason (a constraint error) fails at once.
- **R6, coalescing per file and operation.** The command thread coalesces the commands still waiting in its own queue,
  per (file, operation) pair, not per file alone: a turn is a delta, so keeping only a file's last command would drop
  turns. Commands that name no file (a saved view, the session, a folder) coalesce per operation and key: the session
  keeps the last value, and a view keeps the last command for its name.
  - turns add up, mod 4, as `queue_rotation` does (`crates/catalog/src/queue.rs:103-125`); a sum of zero is still sent,
    so its commands are acked;
  - a rating or a border mark keeps the last value. Every command gets its own ack message, by its id; the commands one
    write carried get the same snapshot. If that write fails, they all fail.
- **R7, a multi-step operation is one command, its probes run first, and its write is one transaction.**
  - *Adding a folder.* It is pending from the click. Normalize and the NTFS checks (`check_folder`,
    `crates/scan/src/lib.rs:281-283`, which calls `refusal::check_root`) run first, on a probe thread of their own: on
    the writer, the command thread or a shared pool, a dead drive would hold up everything behind it. Today
    `pg_library::folders::add_folder` does the probes and `add_root` in one call
    (`crates/library/src/folders.rs:31-34`), for the app and the command-line tool; it is split in two. The probe thread
    posts its result to the Qt thread and does not send the command itself, since it cannot know that the window is
    closing. If the add is still pending and the close's decision is idle, the Qt thread sends the command. Otherwise
    the result is kept and nothing is sent ([Shutdown](#shutdown)). The command's whole work is one short write
    transaction (`add_root`, `crates/catalog/src/roots.rs:39-53`, which also deletes the roots nested in the new one).
    After the commit the command thread queues the scan (`Runner::scan`). A refusal by the probes is that pending item's
    failure: of today's three refusals, the two that come from normalize and `check_folder` are found by the probe and
    never reach an ack; a catalog error is the command's failure. A probe of a dead drive may not return for a long
    time, and the add stays pending meanwhile. Nothing has been written for it, so an unsent add does not hold a close,
    and is dropped if the window closes ([Shutdown](#shutdown)).
  - *Removing a folder.* One command, one transaction (`remove_root`, `crates/catalog/src/roots.rs:58-72`), which
    deletes the root row, sets `missing = 1` over the folder's files, and cascades the root's notices and folder-ledger
    rows by foreign key. Today `remove_folder` reads the roots in a separate statement first
    (`crates/app/src/library.rs:315-322`) and passes the root's path to `Runner::folder_removed`, which stops a scan
    under it. The command drops the outer read: `remove_root` reads the path inside its transaction and returns it, and
    the command thread calls `folder_removed` with it after the commit. The measured cost is 2.3 to 3.6 s for the
    189,519-file root and 0.9 to 1.0 s for the 41,293-file root (debug, one machine; not measured in release). Every
    write queued during it waits for it; none fails. A rating made then stays pending for that long. Whether one writer
    makes it safe to split the removal into shorter transactions is not designed ([Open](#open)).
  - *Rescanning* is not a catalog write. It reads the roots (a point-lane request) and asks the control lane to queue
    the scans (Part 5); until Part 5 the GUI thread queues them itself when the read arrives, as it does today.
  - *A scan begins only for a folder that is still a root.* With the removal a command and the rescan a read followed by
    a request, the two can cross in either order: the roots read can come before the removal's commit, and the scan be
    queued after `folder_removed` has found nothing to stop. Today both calls are synchronous, so they cannot.
    `begin_scan` does not check the folder (`crates/catalog/src/roots.rs:79-100`). It gains the check, inside its
    transaction: when no root is the folder or contains it, it refuses, and the job drops that scan without a notice.
    That closes both orders, where disabling the panel's button would close only one.
- **R8, blocking calls from the GUI thread are refused.** The blocking entry points move into a `providers` module of
  `pg_app`'s facade, which the GUI modules do not call. The desktop's providers call them
  (`crates/desktop/src/providers.rs:44-119`). The facade checks the calling thread's role on every blocking call, in
  every build, and `write` checks it in `pg-catalog`. The image providers' Qt pool threads (`Job`,
  `crates/desktop/cpp/providers.cpp:98-120`) run `Job::run()` and call into Rust, so they are workers and may block. A
  blocking call from the GUI thread logs an error and returns the failure the caller already handles. It does not panic:
  a panic across the cxx-qt bridge aborts the process. Two things are not refused:
  - the session-end wait on the command thread and the preferences writer, through one named facade entry
    ([Shutdown](#shutdown));
  - the main thread's work outside the event loop, where it is not the GUI thread ([The writer](#the-writer), the thread
    role): the open until Part 3, and the shutdown's joins.
- **R9, startup.** No bridge object that reaches the app exists before the app does, and the GUI thread opens no
  catalog. [Startup](#startup) is the design.
- **R10, visibility.** The public surface of `pg_app` is its message types and its facade (the blocking `providers`
  module is part of the facade). The commands and acks cross that boundary, so their payloads must be types the front
  end may name. The catalog data types the front end needs move to `pg-model`, which every crate may depend on:
  `Facets`, `pg_catalog::inbox::Entry` (aliased `InboxEntry` in `crates/app/src/reexport.rs:10`), `FailedFile`,
  `FailedFolder`, `FolderFailure`, and the listing and inbox data. Then `Catalogs`, `Reading`, `Views`, `Edit`,
  `LibraryService` and `Session` become `pub(crate)`, and `reexport.rs` is removed with its allowlist entry in
  `tools/boundaries.py` (line 35). The other items that file passes to the front end move to `pg-model` too: `Change`
  and `Cause` (from `pg_jobs`), `WriteError` (`pg_writeback`), `ThumbCap` (`pg_thumbcache`) and `InboxCounts`
  (`pg_catalog`). `pg-model` is the leaf of `tools/crate-layers.txt` and already holds `FileId`; `jobs`, `library`,
  `writeback`, `app` and `desktop` all list it, so `Change` moves there with no layering violation. Part 6 adds a check
  that no `pub` item of `pg_app` names a `pg-catalog` type. `tools/boundaries.py` checks crate-layer edges, dev edges,
  re-exports and platform `cfg`, not item visibility, so the check is needed.

## Build order

Parts are in dependency order: no part uses what a later one builds, and the app works after each one.

| Part | Builds | Why here |
| --- | --- | --- |
| 1 | the `Database`, the writer, read-only connections, the revision, the command thread | Everything else stands on it: the revision is bumped in one place only once there is one writer. It makes the pool read-only, so the GUI's writes need the command thread at once |
| 2 | the import's pairing outside its transaction; the checkpoint, if measured to be needed | The pairing would otherwise hold the writer |
| 3 | the read side: reads that return their revision, the point lane, R2, the change payloads, startup | A write whose result the GUI cannot read back consistently is worse than a write that waits |
| 4 | the GUI stops waiting: the pending set, the mark, the close | It needs Part 3's revisions |
| 5 | the file, lock and log work that is still on the GUI thread | Independent of the catalog |
| 6 | enforcement and visibility | The checks forbid what Parts 1 to 5 removed |

### Part 1, the `Database`: one writer, read-only connections, the revision

[The writer](#the-writer), [Opening the `Database`](#opening-the-database) and [The command thread](#the-command-thread)
give the design. This part says what moves onto them and what changes on the way.

**Every site that opens a connection or begins a write transaction:**

| Today | Site | Becomes |
| --- | --- | --- |
| the job's connection, kept between rounds | `round`, `crates/library/src/runner.rs` | a read connection; each pass's transactions are `write(job, ...)` |
| the scan | `lease.tx()`, `crates/scan/src/lib.rs:623`; `begin_scan`, `finish_scan`, `mark_missing` and `folder_refused`, `crates/catalog/src/roots.rs:79-163`; the pairing tiers, `crates/catalog/src/pair.rs:266-283` | job writes; the lease is below. `pair_after_scan`, `in_batches` (`pair.rs:79`, `:266-283`) and `mark_missing` (`roots.rs:118-155`) mix reads with several write transactions, so they take a read connection, the writer's handle, the class and the stop |
| the import | `catalog.tx()`, `crates/import/src/lib.rs:51` | one job write per flush (Part 2a) |
| the borders pass | `catalog.tx()`, `crates/borders/src/lib.rs:61` | job writes |
| the thumbnail pass | its own `Catalog::open`, `crates/thumbs/src/lib.rs:202` | a read connection; its only catalog writes are the failure ledger's (`record_failures`, `clear_failures`), which are job writes. The cache's attach stays as it is ([Components](#components)) |
| the write-back | its own connection, `crates/writeback/src/lib.rs:326`; `fault.rs:73-74` | a read connection; `write_done` and the queue's writes are service writes |
| the inbox | its own connection, `crates/app/src/inbox.rs:742` | a read connection; each step stays its own transaction, as today (`inbox.rs:813-849`), and is one service write |
| the listing lane | `crates/app/src/reader.rs:172` | a read connection |
| the GUI's pool writes | `crates/app/src/edit.rs:29`, `:36`; `views.rs:42`, `:52`, `:57`; `library.rs:300`, `:315`, `:338` | commands (below) |
| the pool | `Catalogs::with_catalog`, `crates/app/src/catalogs.rs:38-43` | read connections only |
| the keeper | `pg_library::open_catalog`, `crates/app/src/app.rs:62` | `Database::open`; the writer's connection takes its place |
| the command-line tool | `pg_library::open_catalog`, `crates/cli/src/lib.rs:136` | `Database::open` |

Crops are not in the table: `pg_crop::save` only reads the catalog (`crates/crop/src/lib.rs:68-78`). Outside
`pg-catalog` the calls to `Catalog::open`, `prepare` or `open_catalog`, tests included, number about 30 in `library`, 8
in `app`, 4 each in `cli`, `scan`, `thumbs` and `bench`, and 2 in `writeback`; `pg-catalog` itself has about 90, most in
tests. The counts are from one grep and are a guide to the size of the change, not a list.

**The GUI's writes, until Part 4.** The GUI's calls keep their signatures. Each sends its command to the command thread
with a 5 s deadline as its stop, and waits on the GUI thread for the reply, which comes over a channel (the Qt thread is
blocked, so nothing can be posted to it).

- The reply is definite: the ack, or `Stopped`. A command that was still waiting at 5 s is withdrawn and fails, as a
  busy write fails today. A command whose transaction had begun is waited out, so the GUI never reports a failure for a
  write that then commits.
- The command thread already does what follows a commit (the write-back's wake, the scan, `folder_removed`), on its own
  thread. The GUI's own code after the call is unchanged until Part 3, where the ack replaces it.
- An add's probes stay where they are until Part 4: on the GUI thread, before the command is sent. The command is only
  the `add_root` write.
- Coalescing (R6) is in the command thread from the start but cannot happen yet: the GUI sends one command and waits.
- So the GUI behaves as today after Part 1. The job does not: its writes now wait on another process instead of failing
  after 5 s (Part 2a). Part 4 removes the GUI's wait, and Part 6's check then forbids it.

**Opening, until Part 3.** `app::open` still runs on the main thread, before Qt (`crates/desktop/src/lib.rs:164`).

- Its `Database::open` is given a 5 s deadline as its stop. With the busy handler in place of the timeout (R5), a lock
  held by the command-line tool would otherwise hang the process with no window; with the deadline the open fails as
  today, with the fatal box.
- The main thread is not yet the GUI thread then: it takes that role only for the event loop ([The
  writer](#the-writer)), so `write` does not refuse the preparation's marks.

**The preparation's marks go through the writer.** `Catalog::prepare` writes today, each in its own immediate
transaction: `mark_unheld_missing` and the import, borders, thumbs and taken-zone marks
(`crates/catalog/src/core.rs:115-119`). They run even when another process has the file open; only a rebuild or a
migration needs the exclusive hold (`core.rs:91-95`). In `Database::open` they are the writer's first transactions
([Opening the `Database`](#opening-the-database), step 5). They bump the revision, so once Part 3 is built a running
window is told (R3a). `set_import_version` rewrites every file row twice (`crates/catalog/src/lifecycle.rs:76-84`), so a
second process starting can hold the lock for seconds.

**The scan's lease.** `ScanLease` holds `&mut Catalog` and derefs to it (`crates/catalog/src/core.rs:247-291`); the walk
reads and writes through it. It guards against other processes and crashed walks, with a 60 s expiry renewed every 20 s
(`crates/catalog/src/types.rs:89`, `crates/scan/src/lib.rs:142`), so it is still needed.

- It becomes a value over a read connection and the writer's handle. Its reads use the connection and its writes are job
  writes.
- *Its release is a call, not a `Drop`.* Today `Drop` writes (`core.rs:293-297`). A blocking `write` in a destructor
  would run during an unwind and after the writer has stopped. A lease that is dropped without the call expires by
  itself.
- *A write that waited may have lost the lease.* A job write can now wait behind edits, service writes and another
  process. `renew` already detects a lost lease (`scan/src/lib.rs:600-606`), but the per-batch heartbeat ignores its row
  count (`crates/catalog/src/writers.rs:83`), so a batch can commit after expiry. The heartbeat checks the count inside
  the transaction and rolls the batch back when the lease is gone.

**`Stopped` is not a failure.** It is `WriteError::Stopped`, a variant of its own ([The writer](#the-writer)), so each
caller sees it where it calls `write`, before the error is folded into the caller's own error type.

- *In the job* any `Err` from a pass becomes a notice today: `RoundFailed`, `ScanFailed`, `LedgerNotWritten`
  (`crates/library/src/runner.rs`, by symbol). `Stopped` instead ends the pass as a preemption does
  (`Ok(Step::Preempted)`), with no notice. Two places flatten or misroute an error today and must test for it first:
  - the scan's `ScanError::Failed`, which posts `ScanFailed` and calls `scanned()`;
  - the thumbnail pass, which turns the ledger's error into a `String` inside `pg_thumbs`
    (`crates/thumbs/src/lib.rs:77-78`). `LedgerOutcome` gains a stopped case, which the command-line tool also reads
    (`crates/cli/src/lib.rs:267`). In that case the pass still holds the files' change numbers back (`fold(first_failed,
    ...)`, `thumbs/src/lib.rs:159-163`), or the cache's check would pass over failures that were never recorded.
- *In the write-back* a `write_done` that is stopped after the file was rewritten does not leave the catalog and the
  file apart: the temp is recorded (`crates/writeback/src/lib.rs:579`), and `finish_copy` (`:649`) completes it at the
  next start. But the error branch (`:416-445`) calls `write_failed`, which itself writes, and `run` sets the status
  error (`:307-311`). `Stopped` is neither an attempt nor an error: the write-back ends its work there.

**Writes cannot be made anywhere else.** Two things enforce it, and neither is a grep:

- every other connection is `query_only`, so SQLite refuses the statement;
- the write functions take the transaction, not the connection, and only the writer makes one. A write outside a
  transaction does not compile.

These run on a bare connection today and are converted:

- `set_border_mark` (`crates/catalog/src/borders.rs:22-30`);
- `save_view` and `delete_view` (`crates/catalog/src/browse/views.rs:26-39`), and `save_session_listing`
  (`views.rs:50`);
- the failure ledger (`crates/catalog/src/ledger.rs:250-258`, through `ledger.rs:382-406`). `mark_ledger_read_on` and
  `dismiss_ledger_on` are also called inside the inbox's transaction (`crates/catalog/src/inbox.rs:226`, `:262`), so
  they take the caller's transaction and never begin one;
- the queue's writes: `write_failed`, `drop_gone_write` and `retry_write_copies`
  (`crates/catalog/src/queue.rs:283-309`), and `record_write_temp` (`queue.rs:214`), which sets the "being written"
  state that `IS_CURRENT` reads;
- `finish_scan` (`crates/catalog/src/roots.rs:105`);
- `set_mark` (`crates/catalog/src/lifecycle.rs:105`), which opens its own immediate transaction. It backs the
  preparation's marks, and rewrites `files.pending`, `seq` and `analysis.taken`;
- the scan lease's direct renewal and release (`crates/catalog/src/core.rs:264-271`, `:302-308`);
- the command-line tool's review import, which makes one bare write per row (`crates/cli/src/borders.rs:196`);
- the catalog's own tests that write raw SQL through `c.conn` outside the fixtures (`ledger.rs:529`, `pair.rs:567`,
  `roots.rs:545`, `writers.rs:959-965`), which would fail on a `query_only` connection.

What stays outside the writer: the rebuild's `copy_tables` (`crates/catalog/src/rebuild.rs:186`), which fills a
replacement file under the exclusive hold before the writer exists; the migration step's transaction, which the opening
thread runs on the prepared connection before it moves it to the writer; and the `PRAGMA` statements.

**Tests and fixtures.**

- *Tests use files.* About 30 sites open a `:memory:` catalog (most in `catalog`, 2 in `scan`, 1 in `bench`), among them
  the shared fixture (`crates/catalog/src/test_support.rs:33`). A `:memory:` database has one connection, and the design
  needs a writer's and readers'. Those tests move to catalog files in a temporary folder (`pg_testkit::run_folder`;
  `pg-testkit` is already a dev-dependency of `pg-catalog`). The product code gains no single-connection mode, and loses
  the one it has: the `memory` branches of `core.rs` (`:68-69`, `:92-93`, `:104`, `:124`, `:136`) and the test of that
  mode (`lifecycle.rs:955-957`) go. About 250 catalog tests each gain a folder, a file and a writer thread; the cost is
  not measured.
- *Fixtures write through `write`.* They are gated by `feature = "fixture"`, not `cfg(test)`
  (`crates/catalog/src/lib.rs:21`, `transaction.rs:42`), and the app's tests call them on pooled connections
  (`crates/app/src/images.rs:344-349`, `:401`, `:434`, `:469`; `app.rs:238`).
- *The hold timing moves to the `Database`.* It is a thread-local today (`WATCHED`, `transaction.rs:47-50`; the
  200,000-file scan test asserts on it, `crates/scan/src/lib.rs:1318-1324`), and the holds are now on the writer's
  thread.
- *A test's `Database`* starts a writer thread, and a watcher only when the test gives it a function to report to, as
  R3a's tests do. Two `Database` values on one file in one process work: the second finds the exclusive hold refused,
  and its preparation changes no row when the marks match, so it bumps nothing. That rests on the file locks being held
  per handle, which is not checked for the Linux build.
- The benchmarks' baselines will include the hand-off to the writer.
- The tests of the writer and of the open are under [Tests](#tests).

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

**The revision.** One number for the whole catalog. A sequence kept by the GUI's commands alone could not order the
import's, the scan's and the removal's writes, and a read must be able to say which writes it includes. A migration adds
one `meta` row holding it (one `Step::Sql`; a migration already runs in one write transaction,
`crates/catalog/src/schema.rs:401-435`). `meta` is a key and value table with no triggers on it. The existing `meta.seq`
(`crates/catalog/src/sql.rs:17-19`) is not it: that is the thumbnail clock, bumped by triggers and `set_mark` and read
by the thumbnail cache. An older build refuses a newer schema (`crates/catalog/src/core.rs:75-81`, `:143-148`), so no
process writes the file without bumping, apart from the screenshot fixtures (R3a).

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
- *A rebuild.* `copy_tables` already carries a `meta` key that both files have
  (`crates/catalog/src/rebuild.rs:205-209`), so the revision is carried once the new file's seed has the key. A rebuild
  that carries nothing starts from the seed (`rebuild.rs:73-83`); that is harmless, because a rebuild needs the
  exclusive hold, so no other process and no window is reading.

Part 1 only writes the revision. The reads that return it are built in Part 3, with the read side that uses it.

### Part 2, the import's pairing and the checkpoints

**2a, the pairing is computed outside the write transaction; the index changes only after commit.**

Today the import (`crates/import/src/lib.rs`) runs each 500-file batch as: open the transaction; `store_analysis`;
`pair_and_insert` (query, then insert) inside it; `store_pdq_pair`; commit. With one writer, that transaction would run
on the writer's thread, and its pairing (76 to 92% of the hold) would keep every other write waiting. The hold is the
reason, not correctness: today's import already skips pairing for a rejected file and has no index rollback. The
design replaces it:

- **Scratch split.** `PdqIndex::best_matches` and `query` (`crates/dedup/src/pdq_index.rs:283-345`, both `&mut self`
  today) take `&self` and a caller-owned `QueryScratch { seen: Vec<u32>, query_no: u32, found: Vec<(FileId, u32)> }`.
  Today `seen` and `query_no` are fields of the index (`pdq_index.rs:53-54`, mutated at `:308-322`); `found` is already
  a caller's buffer (`Pairer.found`, `crates/dedup/src/pairer.rs:13`) and moves into the scratch with `Pairer.best`
  (`:15`). `QueryScratch::ensure(len)` sizes `seen` to the index. `Pairer` keeps the index only.
  `Pairer::pairs_in_index(&self, scratch, id, orientations)` returns each neighbour's best (distance, orientation), self
  excluded. `Pairer::insert(&mut self, id, orientations)` is the post-commit insert.
- **Batch matching, on the job's thread, before the write.** The candidates are the batch's files with a usable dihedral
  (today's `quality_ok` test, a local in `crates/import/src/lib.rs:61`), in the batch's own order. That is the order the
  items reached the sink (`crates/jobs/src/pipeline.rs:259`), which is the order today's loop pairs them in. For
  candidate k: its pairs against the index, plus its pairs against each earlier candidate j < k of the same batch. For
  j, a distance `d` between an orientation `v` of k and an orientation of j within `MATCH_DISTANCE` gives j's best `(d,
  v)`: the rule `best_matches` applies to stored entries.
- **The write.** One job write. Store the analyses (`store_analysis`, `crates/catalog/src/writers.rs`, with its
  `IS_CURRENT` test). Store a pair only when both of its files were stored. A rejected file's pairs are then dropped,
  and no other pair changes, because a pair is a relation between two files. `store_analysis` deletes every pair naming
  its file (and so does `store_failure`), so the batch's pairs are written after both files' analyses, in the same
  transaction. The write returns which files were stored.
- **After commit.** Insert the stored candidates' orientations (`Pairer::insert`). The room is checked before the write:
  `u32::try_from` of the index's length plus the orientations must succeed, the u32 link limit. (`reserve()` only grows
  capacity, `pdq_index.rs:199`.) So this insert cannot fail with `IndexFull` (`pdq_index.rs:213`, `:274`). The index
  holds only files that are not pending import (`crates/catalog/src/work.rs:124-126`), so a file being imported is never
  in it already: nothing needs removing and nothing is inserted twice.
- **Equivalence.** Sequential query-then-insert pairs each new file with the earlier stored files. Batch matching gives
  the same pairs for two reasons, and neither is the order test:
  - the index is exact, not approximate (multi-index hashing with `MAX_RADIUS = 31 = MATCH_DISTANCE`, `pdq_index.rs:1`,
    `crates/model/src/pdq.rs:7`), so a brute-force comparison inside the batch finds what a query of the index would.
    `a_built_index_finds_exactly_the_brute_force_set` (`pdq_index.rs:452`) tests that;
  - the later file is the query in both, so the stored orientation is the same. The variant is the query file's
    orientation, ties keep the lowest (`d < b.1`, `pdq_index.rs:295`), and `store_pdq_pair` orders the ids but not the
    variant (`crates/catalog/src/writers.rs`).

  A rejected file's pairs are not stored in either: today a rejected file skips pairing (`import/src/lib.rs:57-60`).
  `pairs_do_not_depend_on_import_order` (`pdq_index.rs:386-414`) does not cover this. It compares only the set of id
  pairs across sequential orders, on `PdqIndex`, without distances or variants.
- **A neighbour that changed.** "Both files stored" covers the batch's own files. A pair with a file already in the
  index is stored even if that file changed between the matching and the commit. Whether today's import has the same
  window, and whether the write should re-check the neighbour, is not traced ([Open](#open)).
- **Failure.** A commit that fails leaves the index unchanged, because nothing was inserted. The run ends at that batch,
  as today; today the index is simply dropped with the run.
- **Busy.** The import has no busy policy of its own. Its write waits in the writer's queue, and the writer waits on
  another process until the job's quit and its grace have passed (R5). Today a busy batch fails after the 5 s timeout
  and ends the run (`pipeline.rs:262`).
- **Cost.** Batch matching is in memory: about 125,000 pairs of 64 orientation comparisons per 500 files. Not measured.
  The writer's hold is the SQL alone: flush minus pairing was 0.19 to 0.56 s per batch (second pass, release).
- **Tests.** A failed commit leaves the index unchanged. A pair naming a rejected file is not stored. A new equivalence
  test: the same files, imported one per batch and all in one batch, store the same pairs with the same distance and
  variant, including two files of one batch that match each other. The order test still holds.

**2b, the checkpoint yields to edits.**

**Precondition.** In each measured run one turn stalled 100 ms or more, at the first 1000-frame checkpoint. Every turn
was timed and no other stalled, so the about 20 checkpoints that should have followed in the same run cost under 100 ms
each, if they ran ([Measurements](#measurements)). What is not known is whether they ran, what each cost, and what the
first one paid for. Measure that first ([Open](#open)). If only the first one costs, SQLite's automatic checkpoint stays
as it is and this part is not built.

If they do stall, an automatic checkpoint runs on the writer's thread after a commit and delays whatever is queued
behind it, an edit included. So the writer decides when, and runs it itself; the checkpoint gets no thread of its own:

- The writer's connection registers a WAL hook (`Connection::wal_hook`, verified in the locked rusqlite revision
  `9d76035` behind the `hooks` feature, `Cargo.toml:36`; it takes `&mut Connection`). Registering a hook replaces
  SQLite's default hook, which is the automatic checkpoint (`sqlite3_wal_autocheckpoint` is itself a `sqlite3_wal_hook`
  call in the bundled `sqlite3.c`), so no pragma is needed. The hook runs only on a commit. It receives the frame count
  and, at 1000 frames or more (SQLite's default), marks a checkpoint as due. It does no SQL and always returns `Ok(())`,
  since an error becomes an SQLite error code.
- Read connections never commit a write, so they never run an automatic checkpoint, and they are closed by drop
  ([Design](#design)), so they never run the close's `TRUNCATE` either. The rebuild's connection has no hook; it ends
  with `wal_checkpoint(TRUNCATE)` and a close (`crates/catalog/src/rebuild.rs:119-120`).
- *When it runs.* A due checkpoint runs once, as `PRAGMA wal_checkpoint(PASSIVE)`, as soon as no edit is queued: ahead
  of the next service or job write, or at once if the queue is empty. Running it clears the mark, whatever it achieved;
  only a later commit can set the mark again. So an idle writer does not checkpoint in a loop, and a session of edits
  alone still checkpoints when the edits pause. An edit that arrives during the checkpoint waits for it. A passive
  checkpoint reports "busy" as a column of its result row, not as an error, so the writer reads the row.
- *The bound is the automatic checkpoint's, no better.* The writer is the only one in this process that appends, and it
  is not appending while it checkpoints. But a passive checkpoint cannot pass a frame that a reader's snapshot still
  needs, and the WAL restarts only when no reader is in it. A reader that holds one snapshot for long pins it: the
  thumbnail cache's pending query holds one read transaction over both files (`crates/thumbcache/src/reader.rs:29-62`),
  and a pass that keeps a cursor open across its own writes would do the same. Another process appends too. All of that
  is true of the automatic checkpoint today.
- A file size is not used as the trigger, because the WAL file keeps its size after a checkpoint, so a size test would
  fire on every commit.
- The close still runs `TRUNCATE` (`crates/catalog/src/core.rs:52-53`), on the writer's connection, last
  ([Shutdown](#shutdown)).
- Measured (first observations; see [Measurements](#measurements)): with auto-checkpoint on, a commit at 1000 frames
  stalled 0.51 to 0.84 s, in four runs. With it off, the slowest of 3,000 turns took 3 to 5 ms, and the WAL grew to 90
  MB.
- **Tests.** Both read the frame count the hook reports, not the file size, which never shrinks. Both run with no other
  reader open.
  - Commits of a few frames each cross 1,000 frames with the queue empty. The commit after the checkpoint reports a
    frame count below 1,000 (the WAL restarted).
  - An edit-class write is queued when the count crosses 1,000. It returns before the checkpoint runs.

### Part 3, the read side

- **Reads that return their revision.** A read whose result reaches the GUI's stored layer returns the revision it was
  read at, from the same snapshot as its data. `Catalog` gains `snapshot(|s| ...)`: it begins a deferred read
  transaction, reads the revision row, runs the closure, and returns the closure's result with the revision. A read-only
  transaction is not blocked by a writer in WAL mode.
  - *Which reads.* The point lane's (details, entry, views, roots) and the listing lane's kinds. The providers' reads
    (`file_state`, `file_source`, `placeholder`, `crates/app/src/images.rs:151`, `:163`, `:270`) and `file_path` feed no
    stored value and stay as they are.
  - *What changes in them.* Today's reads have three shapes. One autocommit statement (`grid_entry`,
    `crates/catalog/src/browse/read/grid.rs:100-106`) simply runs inside the snapshot. Two autocommit statements, which
    are two snapshots today (`folder_tree`, `browse/read/tree.rs:24-40`), become one. A read that begins its own
    deferred transaction (`details`, `details.rs:14`; `list_grid`, `grid.rs:118`; `inbox_entries`, `status`,
    `pending_import`) cannot be wrapped, since a transaction cannot begin inside another; those among the lanes' reads
    take the snapshot as their argument and no longer begin one.
  - *The cost* is a `BEGIN`, one more row read and a `COMMIT` around each point read, which today takes 0.02 to 0.12 ms.
    It is not measured ([Open](#open)).
- **Point lane**: as in [Components](#components). Each request runs in one `snapshot`.
- **Listing lane**: the existing `Reader`, unchanged in behaviour; each kind's closure runs in one `snapshot`.
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
    `shell.rs:497`). Without the resolve a failure notice would never clear after a later success. No QML caller reads a
    return value from these calls, so nothing else depends on them being synchronous.
  - *The synchronous re-reads go.* `fresh_entry` in `Grid::arrive` is removed: the reply applied as an ack, and the
    notes, cover the edits made while a list loaded. The one in the controller's `Command::RefreshTurned`
    (`desktop/src/gallery/edits.rs:58`) is removed too: the file changes' set (below) asks the point lane, and
    `RefreshTurned` stays as a controller command that reads nothing, so recordings still replay.
  - *An empty slot.* The grid's selection has one slot. While its details are on the way, the selection's line shows
    what it shows today without details: the rating symbol alone. The full line needs the details' path and size
    (`gallery/items.rs:193`).
  - *Applying many results.* `Grid::position_of` is a linear scan (`crates/client/src/grid.rs:168-170`), and each
    applied entry runs `bump_revision` (`gallery/model.rs:256-261`). The entries of one Qt pass are applied together,
    with one bump, and the grid gains an index from id to position, so that thousands of acks arriving after a long wait
    do not cost a scan each.
- **The bus, the pump and the watcher's route.** `App::open` creates the change bus and gives the `Database` the
  functions that publish `Change::Foreign` (R3a) and report the busy state (R5).
  - *One pump for the process, not one per `Library` object.* Today the pump is bound to a `Library` object, exists only
    after `library().start()` succeeds (`desktop/src/library.rs:105-122`), runs from `startHooks` after the first list
    (`Main.qml:752-757`), and is not made again for a window rebuilt by a live reload, since `start` returns early once
    the library runs (`library.rs:101-103`). The desktop instead subscribes one pump at Ready, in Rust, before the main
    content loads, so before the first reads of `Main.qml:745-747`. It hands its passes to whichever `Library` object is
    registered, as acks are handed on ([The command thread](#the-command-thread)). So no change is missed between the
    first list and the job's start, edits are seen even when the library fails to start, and a reloaded window keeps its
    pump.
  - *The status.* The pump's thread reads `library().status()` when an event arrives and hands it to the Qt pass with
    the folded changes ([Inventory](#inventory)). The first status has no event to carry it; it comes from the startup
    worker's slot ([Startup](#startup)).
- **Change payloads.** The bus carries no payload today. `Change` is a fieldless enum
  (`crates/jobs/src/events.rs:127-141`: `Progress`, `Scanned`, `Imported`, `Round`, `Writes`, `Turned`), handed out as
  `Bus<Change>` by `Library::changes` (`crates/app/src/library.rs:160`). The bus holds 256 events per subscriber
  (`library.rs:99`). The design keeps `Change` `Copy`, which it derives today, so the bus can clone it cheaply.
  - `Change::Turned { file: FileId, revision: u64 }`. Today `Turned` names no file
    (`crates/writeback/src/lib.rs:207-209`), and the write-back publishes it once per file. Verified: the write-back's
    `job` is in scope at all three call sites of `pixels_changed` (`crates/writeback/src/lib.rs:611`, `:679`, `:695`),
    so `pixels_changed` takes `job.file_id`. The revision is the one the write-back's `write_done`
    (`crates/catalog/src/queue.rs:224-229`) returned from the writer; it precedes each call. At `:611` it runs inside
    `copy_and_done`, which returns `(i64, i64)` today, so the revision is threaded out of it. The desktop match
    (`desktop/src/library.rs:57`) changes to `Turned { .. }`.
  - `Change::Rewritten { file: FileId, revision: u64 }`, new. The write-back's rewrite of a rating into the file also
    changes the file's row (`file_rewritten`, `queue.rs:231-234`): its `mtime`, which is in the image key
    (`gallery/items.rs:45`), and the details' `rating_in_file`. Today it publishes only `Writes`, and both stay stale
    until a re-list. It publishes `Rewritten` as well, and R2 reads the entry and the details again.
  - `Change::Foreign { revision: u64 }`, from the watcher's route (R3a): refresh everything.
  - *No event for a command.* The window that sent it gets the ack, which carries the new value and feeds R2's notes,
    and there is one window. So a rating never goes through the pump, and never causes a re-list (R4).
  - `Scanned`, `Imported`, `Round` and `Writes` keep their handling and stay fieldless: `Scanned` reloads the tree and
    the grid, and `Imported` re-lists, once per pass (`Main.qml:127-128`, `:255-265`; `crates/library/src/runner.rs`,
    `Imported`). Their reloads go through the listing lane, whose results carry their own revision. The job's per-batch
    commits publish nothing more.
  - `Progress` is unchanged.
- **Pumps.** A pump waits with `recv()` for its first event, then calls `take()`. It handles the event it received, then
  everything `take()` returns, in order, then the lag count. `take()` drains the queue, so its events are not discarded.
  `take()` alone does not wait, so it cannot be the wait. For the library pump a non-zero lag count is the R3 refresh;
  the notices pump logs it; the Yandex pump's events carry nothing (`desktop/src/yandex.rs:127-128`: its change is `()`,
  since a poll reads the whole state), so it discards them and the count.
  - *Today no pump reads the lag count.* The library pump takes one event per `recv()` and folds it into flags
    (`desktop/src/library.rs:114-122`, `pump.rs:51-64`); the notices pump does the same on its own thread
    (`notices.rs:260-277`); the Yandex pump discards what `take()` returns. `take()` is the only reader of the count,
    and reading it resets it (`events.rs:103-106`). The log panel's pump uses an unbounded channel and has no lag
    (`log_model.rs:124`). `pump::start`'s `recv: FnMut() -> Option<C>` changes to return a batch and the lag count,
    which changes the Yandex pump's call too (`yandex.rs:126-130`).
  - *Folding the file changes.* The library pump's flags (`Changes`, under `Pending`, `desktop/src/library.rs:43-62`,
    `pump.rs:14-25`) gain a set of (file, revision) for `Turned` and `Rewritten`. Above 256 files in one pass the set
    collapses to "refresh everything".
  - *The set's route to the grid.* Today the pump reaches the grid through a QML signal and the controller
    (`Main.qml:130`, `Event::FilesChanged`, the controller's `Command::RefreshTurned`, `gallery::post`;
    `crates/client/src/controller.rs:327`, `desktop/src/shell.rs:495`), a chain that carries no payload and is recorded
    for replay. The set does not go through it: it is handed to the gallery model in Rust (`gallery::post`, which takes
    any closure), and the signal stays as it is.
- **Read-ahead.** When the viewer's open item or the grid's selection changes, the GUI queues point-lane reads for the
  slots that changed. Their results apply under R2. A details or entry read measured 0.08 to 0.12 ms (p50) and at most
  0.39 ms.
- **The point-lane conversions.** The reads the Inventory targets at the point lane move to it here ([Catalog
  reads](#catalog-reads-on-the-gui-thread--point-lane)). A path for a video or an animation that arrives after the
  viewer has moved on (`MediaView.qml:57-64`) is dropped.
- **Startup.** All of [Startup](#startup) is built here: the startup worker, the slot, `Startup`, the `Loader`, the busy
  function passed into `app::open` and its holder (R5), the failed open and the close before Ready. `app::open` and the
  start state's reads move to the worker, so the GUI thread opens no catalog, and Part 1's 5 s deadline on the open is
  replaced by the worker's stop.
- **The screenshot harness.** Its settle check (`Shell::pending`, `shell.rs:242-244`) gains "no point-lane read in
  flight", as `failed_files::loading()` already is there: a details read that lands late changes the status line. The
  goldens that rate and turn depend on it (`viewer-turn`, `viewer-rate`, `tools/compare_shots.py:113-114`).

### Part 4, the GUI stops waiting

Ratings, turns, border marks, saved views, the session and the folders are already commands (Part 1). Here the GUI stops
waiting for them: it sends, shows the pending set, and takes the ack when it is posted to the Qt thread (R4 to R7). An
add's probes run before the command is sent, on a probe thread (R7). A rescan is not a command (R7).

What the Qt side gains:

- the pending set, with the cap (R5), and the one place that receives acks ([The command thread](#the-command-thread)).
  Coalescing (R6) can now happen, since the GUI no longer waits;
- every reader of an edited field reading the shown value (R4);
- the busy state's property and its transient toast (R5). The status bar's toast is the inbox's today (`Main.qml:486`),
  so this one needs a path of its own;
- pending adds and removals in the library panel (R4): the dimmed row, the separate lines, and the empty-state texts
  while the first add is pending;
- the mark of a pending value. The grid and the viewer have no `Accessible` properties at all today (only `LogPanel`,
  `StatusBar` and `InboxPanel` do), and a row's roles (`GridRow.qml:16-28`) have none for "pending". Both are new: a
  pending role on the row and in the viewer, an accessible description from it, and the one 200 ms timer, which needs a
  timer of its own: the existing one is the `Shell`'s deadline (`deadline.rs`). The QML tests' stubs
  (`crates/desktop/tests/qml/tst_gridrow.qml`, `tst_viewer.qml`, `stubs/`) gain the role;
- `Grid.bordered`, which is maintained incrementally (`crates/client/src/grid.rs:323-342`), derived from both layers;
- the close's decision, the count of posted model commands, the three controller events (closing, keep-working,
  save-now), the session-end handler and the quit request ([Shutdown](#shutdown));
- the screenshot harness's settle check gains "the pending set is empty and no model command is posted", and its quits
  request the quit first ([Shutdown](#shutdown)).

The shutdown's steps themselves land with the part that builds each thread: the command thread's stop and the
`Database`'s close in Part 1, the point lane's in Part 3, Part 5's workers', the preferences writer's and the log
thread's in Part 5.

### Part 5, the worker items

The file, lock and log work that the GUI thread still does moves off it. See [Not the
catalog](#not-the-catalog-but-on-the-gui-thread--worker-part-5). Three kinds of worker take it:

| Worker | For | Order |
| --- | --- | --- |
| a probe thread, one per probe | a file or volume probe that can hang on a dead drive: an add's checks (R7), the picker's start folder | none: a probe is never queued behind another and holds nothing else back. One that never returns is abandoned at exit |
| the control lane, one thread | every request that changes the library job: the thumbnail height, pause, the saving of crops, a wake, a rescan's scans | in the order the GUI made them. On a pool, two heights or a pause and an unpause could be applied reversed: the height is last-writer-wins (`crates/library/src/runner.rs`, `HeightCell::set`) and pause is a toggle (`desktop/src/library.rs:245-249`) |
| the pool | blocking work that depends on nothing else: joining an old animation player, opening a video's file, the non-native picker's folder listing | none |

The session-end handler's wait on the preferences writer is added here, with that writer.

### Tests

Part 2's tests are stated with it. The others:

- **The writer** (Part 1), on a `Database` over a file in a temporary folder:
  - with the writer held by a slow closure, queued writes are served edits first, then service, then job, and first in
    first out inside a class;
  - a waiting call whose stop is raised returns `Stopped` and its closure never runs; a call whose closure has begun
    commits although its stop is raised;
  - a waiting job write returns `Stopped` 4 s after the quit is raised, not at once;
  - a closure that panics rolls back, the panic resumes on the caller, and the next write succeeds on the same
    connection;
  - a closure that changes a row bumps the revision once; one that changes none does not bump, and `data_version` on
    another connection does not move;
  - a `write` from the GUI role and one from inside a closure are refused;
  - a write on a read connection fails;
  - a scan batch whose lease has expired rolls back.
- **Opening** (Part 1): with another `Database` holding the write lock, an open whose stop is a 5 s deadline returns
  `Stopped`; the marks of a second open on the same file bump nothing when they match.
- **R2** (Part 3), on `pg_client::Grid` and the plain types, with results fed in a chosen order:
  - an older point read that arrives after a newer ack is refused;
  - an older listing applied after an ack keeps the acked entry, and the entry stays in the sparse map;
  - a file that turns while a listing containing it is in flight is asked for again when the listing lands;
  - a listing held under the viewer keeps the notes until it is applied;
  - a failed listing shows empty and changes no revision; a failed tree is the empty tree and still reports loaded;
  - an applied listing asks again for each details slot older than it;
  - a roots read older than a removal's ack is refused.
- **R3 and R3a** (Part 3): two causes inside 5 s give one refresh after the first; a commit by a second `Database` on
  the same file, standing in for another process, is reported once; this process's own commit is not reported; a foreign
  commit followed at once by an own commit is still reported; a foreign commit seen at the start of a long transaction
  is reported before that transaction ends.
- **The interim reply** (Part 3): a turn through the blocking reply is applied once, and `scanned()` fires once on a
  removal.
- **Startup** (Part 3): a close before Ready ends in the ordinary shutdown whatever the worker reports; a stopped open
  shows no box.
- **A scan for a folder that is no longer a root** (Part 4) is refused, in both orders of the rescan and the removal.
- **Part 4**, on the plain types ([Qt-side values](#qt-side-values)) and the controller, with virtual time:
  - a pending rating shows at once and is marked after 200 ms; a failed command restores the stored value;
  - a second border toggle while the first is pending sends "unmark";
  - the cap refuses the 10,001st file command and not a session save;
  - the close's decision, one test for each filled cell of its table ([Closing the window](#closing-the-window));
  - while closing, the controller returns no session save; the keep-working event clears the closing flag, arms the
    session save, and fires again only the timers the flush drained: with no query waiting it makes no listing;
  - the save-now event returns the session save when only the query's timer is waiting, and leaves that timer and the
    layout's armed;
  - session end, one test for each row of its table.
- **Part 5:** two preferences saves queued together leave the newer on disk; the log thread's lines are on disk after a
  normal exit and after a panic; two thumbnail heights sent in a row are applied in that order.

### Part 6, enforcement and visibility

- The blocking entry points move into the facade's `providers` module, and the facade's thread check is added, in every
  build (R8). `write`'s own check is built in Part 1.
- R10's moves to `pg-model`, the removal of `reexport.rs`, and its check: no `pub` item of `pg_app` names a `pg-catalog`
  type.
- Part 1's rule needs no check of its own: a read connection refuses a write, and a write function needs a transaction.
  A test opens a read connection and asserts that a write on it fails.

## Shutdown

### Closing the window

The close itself sends commands: `onClosing` runs `shell.flush()` (`desktop/qml/Main.qml:138`), which saves the saved
session and the last file (`crates/client/src/controller.rs:509-535`), written synchronously today
(`desktop/src/shell.rs:496`). The flush also drains the timers: it fires the row-height save, turns a query typed in the
last 300 ms into a save with no listing, and drops a layout waiting on a resize (`controller.rs:509-523`,
`crates/client/src/controller/deadlines.rs:64-68`). That is right for a window that is going away, and only for that.

**Every way out goes through `onClosing`.** A quit is not a way around it: Qt's handling of the quit event closes every
top-level window and drops the quit if one refuses (`qguiapplication.cpp:2119-2130` in the Qt source). That covers
`QCoreApplication::quit`, which the screenshot harness calls (`cpp/screenshot.cpp:186`, `:211`), and the quit Qt makes
itself when the system's session really ends (`qwindowscontext.cpp:1159-1175`). Today nobody sees it, because
`onClosing` always accepts. So the close's decision must know when a quit, not the user, is asking.

**The close's decision** is one plain value. `onClosing` reads it, and nothing else decides whether the window may
close. If each close decided again from the pending set, the window's own second close would be refused, and "quit
anyway" would never close it.

| State | Meaning |
| --- | --- |
| idle | no close is under way |
| waiting | the user asked to close, and something holds it; input is blocked |
| complete | the window may close; the next `onClosing` accepts |

*What holds a close.* The window waits, because a drain with a deadline after it has gone would lose edits silently. Two
things hold it: a command that has been sent and is neither acked nor failed; and an edit that has been dispatched but
whose command is not yet made. Rate, turn and the border mark are made in a closure queued to the model
(`shell.rs:477-494`, `gallery.rs:61-64`), so between the dispatch and that closure the pending set does not hold them
yet. A count of such closures stands beside the pending set. It is a guard carried inside the closure and released when
the closure is dropped, run or not: `gallery::post` discards a closure when there is no model and ignores a failed queue
(`gallery.rs:64-66`), and a live reload destroys the model; a count lowered only by a run could stay above zero for
good. An add whose command has not been sent holds nothing: its probe may never return (R7).

| Event | idle | waiting | complete |
| --- | --- | --- | --- |
| `onClosing` | flush, then the closing event. Nothing holds: complete, accept. Otherwise: waiting, refuse (`close.accepted = false`, which the code does not use today) | refuse, no flush | accept, no flush |
| nothing holds any more | | complete; the window closes itself | |
| a file, view or folder command fails | its notice, as today | back to idle, by the keep-working event | nothing |
| the session's save fails | its notice, as today | it stops holding; nothing else | nothing |
| "quit anyway" | | complete; the window closes itself | |
| "keep working" | | back to idle | |
| a probe's result for an unsent add | the command is sent, or the refusal shown | kept | kept |
| a quit is requested (harness, session's real end) | complete | complete | complete |
| `commitDataRequest` (session end asked) | save-now, then block up to 3 s | block up to 3 s | nothing |
| a live reload | | back to idle, by the keep-working event | |

Complete is final: nothing takes a complete close back.

**While waiting:**

- *The controller returns no session save.* The closing state is a flag inside the recorded controller, set by the
  closing event and cleared by the keep-working event. `onClosing` sends the closing event after the flush, in that
  order: the flush's own session save must pass before the gate shuts. The session save is re-armed a second after the
  current item changes (`controller.rs:292`), and a list that arrives changes it with no input at all (`apply`,
  `set_current_item`, `onCurrentChanged`, `Main.qml:137`). The controller also returns a session save with no timer when
  a tree result lands (`Event::FolderSpelled`, `controller.rs:313-316`), at the start gate with an applied view
  (`:359-363`), and for a listing change (`listing_event`, `:495`). So the gate is where the controller's `update`
  returns `Command::SaveSession`: while closing, it returns none. What those would have saved is not a user's edit, and
  is not saved if the window closes.
- *Input is blocked* by an overlay like `ModalOverlay.qml`, but not that one as it is: it dismisses on Escape and on a
  click outside (`ModalOverlay.qml:22`, `:30`) and dims as soon as it opens. This one cannot be dismissed. The three
  application shortcuts bypass an overlay (`GlobalShortcuts.qml:24-41`), so the saving state is a `PanelKind`, which
  makes the controller swallow them; a `PanelKind` is recorded but not saved in the session.
- *The "saving" state shows only after 200 ms.* The flush sends the session save when one was waiting, and it commits in
  milliseconds. Shown at once, the state would flash on such a close. After 200 ms the overlay dims and shows two
  choices, "quit anyway" and "keep working".
- The job and the write-back are still running, so a late turn is written.

**The keep-working event** takes the close back. Without it, a catalog held by the command-line tool for minutes would
leave the window blocked with only the choice that loses the edits. It undoes what the request did:

- the decision returns to idle, and the closing flag and the saving panel are cleared; the pending commands stay
  pending;
- the session save is armed, since saves were suppressed meanwhile;
- the query's timer and the layout's are fired again, each only if the flush drained it. The controller notes that at
  the flush (`controller.rs:512-517`). A listing made without a waiting query would show from the top
  (`open_list(false)`, `controller.rs:155-156`, `:549-551`) and lose the scroll position and the selection, so none is
  made. The flush's other two, the row-height save and the session save, were carried out and are not repeated;
- a kept probe result is acted on now: the add's command is sent, or its refusal shown.

Three things send it: the "keep working" choice; a file, view or folder command that fails while the decision is
waiting, so that its failure notice is seen; and the `Shell`'s `cxx_qt::Initialize` after a live reload, when it finds
the controller closing (`ShellRust::default` cannot dispatch, `shell.rs:36-56`; `GalleryModel` already has an
`Initialize`, `gallery.rs:70`). A failed session save does not send it: it is not a user's edit, and taking the close
back for it would loop, because the event arms the session save again and the next close's flush would send it, to fail
within milliseconds, before "quit anyway" is ever shown.

**A quit is requested** by a call that sets the decision to complete before the quit is made, so that the `onClosing`
the quit causes accepts:

- the screenshot harness calls a `Shell` invokable before its `QCoreApplication::quit` (`cpp/screenshot.cpp:186`,
  `:211`). Its `exit(1)` at `:174` does not go through the quit event;
- at the session's real end Qt emits `aboutToQuit` and then quits (`qwindowscontext.cpp:1167-1174`); the app's handler
  of `aboutToQuit` sets it.

**When the window closes.** An add whose command was never sent is dropped, and the `Shell` logs it: nothing was written
for it. It is dropped only then: dropped at the close's request, it would be lost even if the user then chose to keep
working. Whatever the command thread still holds is dropped and logged by that thread (step 1 below).

**The stated guarantee.** An edit the user made is lost at exit only in these cases: an add whose command had not been
sent when the window closed; the user chose "quit anyway"; the operating system ended the session before it was written
(below); or the process was killed. The first two are logged. At session end, what the command thread still holds is
logged, though the line may not reach the file, since the process can be killed right after; an unsent add, or an edit
whose model closure has not run, is not logged then, because no thread but the blocked one knows of it. Not covered,
because they are not commands: a notice's dismiss, mark or retry, which goes through the inbox ([Inventory](#inventory))
and is abandoned if it cannot be written within the inbox's 5 s drain, as today; and a request still queued for Part 5's
workers, such as the saving of cropped copies.

### The operating system ending the session

The app is a plain `QGuiApplication` (`cpp/gui_app.cpp:77`). The system first asks: Qt emits `commitDataRequest` on the
GUI thread, inside its handler for the query, and closes no window. Windows can cancel the session end after that; Qt
then does nothing. If it is not cancelled, Qt emits `aboutToQuit` and quits (`qwindowscontext.cpp:1139-1176`). So the
handler of `commitDataRequest` must leave the window able to go on as before.

| The decision | What the handler does |
| --- | --- |
| before Ready | nothing. There is nothing to save, and stopping the open would leave a cancelled session end with a window that cannot start |
| idle | sends the controller a save-now event, then blocks |
| waiting | blocks. The close's flush has run |
| complete | nothing. The window is already closing |

- *The save-now event* is not the flush. It returns the session save, with the last-file save that goes with it
  (`controller.rs:532-535`), when the session's timer or the query's is waiting: the listing already holds a query typed
  in the last 300 ms (`controller.rs:301-304`), so it is saved, and its timer stays armed so it is still listed if the
  session goes on. It fires the row-height save. It leaves the layout's timer armed. It sets no closing flag and drops
  no add.
- *The block* is on the command thread, until its queue is empty and nothing is in progress, and on the preferences
  writer's drain (the last-file save is a preferences write, `shell.rs:99-102`), for at most 3 s together. It does not
  wait for the pending set: acks are posted to the Qt thread, which is the thread that is blocked. What the command
  thread still holds when the 3 s end, it logs.
- Blocking in that slot is permitted. The process may be killed right after, so the ordinary shutdown is not relied on.
  The 3 s is chosen to stay under the system's own limit, which is not checked ([Open](#open)).
- This wait is an accepted exception to the goal, and one named facade entry is exempt from R8's check for it.
- *The plumbing.* A direct connection on the application object calls an invokable of the current `Shell` synchronously,
  through the root's `shellCore`, as the screenshot harness already does for `pending` (`cpp/screenshot.cpp:82-84`).
  Only the connections are new: this one and `aboutToQuit`'s.

### Other ways out

- *The screenshot harness.* Its settle check does not see pending commands or the mark's timer (`screenshot.cpp:79-112`,
  `shell.rs:242-244`); the check gains "the pending set is empty and no model command is posted". When it quits on a
  timeout with something still pending, it requests the quit first (above), and step 1 below drops and logs what the
  command thread holds.
- *The debug build's live reload* deletes the root (`cpp/live_reload.cpp:95`) and runs no shutdown. Commands already
  sent still run on the command thread, and their acks reach the rebuilt window ([The command
  thread](#the-command-thread)). That is accepted for a debug build.

### The shutdown's steps

Today `crates/desktop/src/lib.rs:212-214` runs `app::stop()`, then `drop(engine)` (which joins the image providers'
pools), then `app::shutdown()`. Two things order the new steps:

- the writer must outlive everything that writes through it;
- each step raises the stop that its thread's `write` calls carry. A call that is waiting then ends with `Stopped`, at
  once or, for the job and the write-back, when their 4 s grace has passed. A transaction that has begun runs to its
  commit.

The event loop has returned by now, and the main thread has given up the GUI role, so its joins are not refused (R8).

| Step | What stops | Notes |
| --- | --- | --- |
| 1 | the command thread | Its queue is empty, or the quit did not wait for it (the user chose "quit anyway", the session ended, or the harness quit), and what is left is dropped and logged |
| 2 | `app::stop()`, as today (`crates/app/src/app.rs:148-154`): the Yandex search (up to 2 s), the listing lane, then the library (the write-back, then the job, whose quit flag is raised here) | Below |
| 3 | `drop(engine)`: the providers finish and their pools join (`crates/desktop/cpp/providers.cpp:131-136`) | The providers only read, so they may run through steps 1 and 2. The pumps' threads are detached (`pump.rs`), and their passes are queued to a Qt thread that no longer runs its loop, so they do nothing more |
| 4 | the point lane and Part 5's workers | The point lane answers its queued requests with an error. The control lane and the pool drop what they have not started. A probe thread that has not returned is abandoned |
| 5 | `app::shutdown` (`app.rs:160-177`): the thumbnail cache (`thumbs.close()`, `:164`), the event forwarder (`:166-170`), the preferences writer, the inbox (`crates/app/src/inbox.rs:454-463`), the pool of read connections | The preferences writer drains first. The inbox abandons what it cannot write within its 5 s drain. The pool's connections are dropped |
| 6 | the `Database`: the watcher, then the writer | Below |
| 7 | the log thread | It flushes and ends last |

- *Step 2 and the write-back's slot mutex.* `Library::stop` holds that mutex while it joins the write-back
  (`crates/app/src/library.rs:166-171`). Nothing that holds it calls `write`: the command thread, which takes it to wake
  the write-back, has stopped. The pump's `status()` takes it too, and `tend` under it can restart a dead write-back
  (`library.rs:125`); neither writes to the catalog while holding it.
- *Step 6.* The watcher stops first, so it calls nothing that has closed. The writer finishes a transaction that is
  still running. That can only be the job's, when the job was abandoned after its 5 s stop wait
  (`crates/library/src/runner.rs`, `stop`; `library.rs:172`): it was inside a long closure, or it began a write just
  then because it found the writer free. Then the writer runs `wal_checkpoint(TRUNCATE)` and closes its connection.
  `TRUNCATE` waits for readers through the busy handler, which gives up after 5 s when no call is current ([The
  writer](#the-writer)), as the timeout does today.
- *The last connection.* The read connections belong to their threads and are dropped as those end, before step 6. Then
  the writer's is the last one, and its close removes the WAL file, as the keeper's does today
  (`crates/app/src/catalogs.rs:53-61`). An abandoned job thread still holds its own; then the WAL file stays until the
  next open, which is safe. A `write` that thread makes later returns `Stopped`.

The facade returns an error at once once stop has begun, so no call made during shutdown can hang.

**The bound.** The joins run on the main thread after the window has closed, when no UI remains to block.

| Wait | Bound |
| --- | --- |
| the Yandex stop | 2 s (`STOP_WAIT`, `crates/app/src/yandex.rs:102`) |
| the job's stop | 5 s (`JOB_STOP_WAIT`, `crates/app/src/library.rs:32`) |
| the inbox's drain | 5 s |
| together | about 12 s |

Not in that bound, and not measured: the listing lane's stop; `~Provider`'s `waitForDone`, which has no bound
(`providers.cpp:135`), while a Yandex pool job can be in a network fetch (`crates/desktop/src/providers.rs:99`); the
write-back's join, whose waiting `write` calls end within the 4 s grace but whose image write in progress does not (it
comes before the job's quit is raised, so the two graces can follow one another); an abandoned job's transaction;
`thumbs.close()`; the forwarder's join; the preferences writer's drain; the command thread's, the point lane's, the
control lane's, the pool's and the watcher's joins; the writer's `TRUNCATE`, which gives up after 5 s; and the log
thread's flush. The bound is accepted for the listed waits only.

## Inventory

Every site on the GUI thread today that touches the catalog, a file or a lock, and where it goes. A path with no crate
is under `crates/desktop/src`.

### Already off the GUI thread

- **The `Reader`** (asynchronous, delivered on the GUI thread): `gallery/model.rs` `open_listing` (`Session::grid`),
  `filters.rs` `count_facets` (`Session::facets`), `folders.rs` `reload` (`Session::tree`), `failed_files.rs` `show`
  (`Session::failed_files`), `notices.rs` `copy` (`Session::copy_details`), and `shell.rs` `reveal`
  (`Session::reveal_folder`, which replaced `Catalogs::present_folder`).
- **Queued to the inbox's thread**: `app.rs` `post`, `resolved`, `resolved_part`; `notices.rs` (marks, `set_viewing`,
  `dismiss`, `dismiss_all`, `retry`, `poke`); `library.rs` `on_pass` (`poke` after the job's rounds). `Inbox::post` is
  not free of GUI work: it logs synchronously first (`crates/app/src/inbox.rs:341-343`), taking the inbox's `logged`
  mutex as it does (`inbox.rs:495`). Part 5's log thread takes that over.
- **Queued to the job or the write-back**: `library().wake_job()` in `notices.rs` `retry` (a closure the inbox thread
  runs once the retry is written, `inbox.rs:408-412`, so not a GUI call), and `library().start` (`library.rs` `start`:
  it starts the job and write-back threads and queues two inbox clears; it opens no catalog itself).

### In memory, stays on the GUI thread

| Call | What it takes | Why it stays |
| --- | --- | --- |
| `prefs().get()` | a clone under the store's lock (`crates/app/src/store.rs:46`) | Part 5 keeps that lock free of I/O |
| `library().running()` | the started library from a cell | no I/O |
| `inbox().subscribe()` | the `first` mutex (`inbox.rs:448-450`) | held across no I/O |
| the Yandex `search().subscribe()` (`yandex.rs:120`) | its bus's mutex | held across no I/O |
| `inbox().idle()` | `shared.work` (`inbox.rs:467`) | the inbox thread holds it only around its condvar wait and its queue bookkeeping (`:776-784`, `:879-890`, `:914`), never across SQL |
| `inbox().listing()` | a separate `listing` mutex (`inbox.rs:443`) | the inbox thread holds it only for a pointer swap (`:959`) |

The Goal accepts a mutex that no thread holds across I/O or a wait.

### Catalog reads on the GUI thread → point lane

| Site | Call | Reached from | Target |
| --- | --- | --- | --- |
| `gallery/state.rs` `with_details` | `reading().details` | `gallery/model.rs` `refresh_viewed` (from `set_viewing_holding` and `bump_revision`) and `refresh_derived` (`model.rs:264-272`); the `info` and `border_rejected` callers are Rust, not QML | point lane, into a details slot ([Qt-side values](#qt-side-values)) |
| `gallery/state.rs` `fresh_entry` | `reading().entry` | `Grid::arrive` (`crates/client/src/grid.rs:289`, for edited ids) and `refresh_turned` (`gallery/edits.rs`) | removed in Part 3. For `arrive`: the reply applied as an ack, and R2's notes (below). For `refresh_turned`: the point lane's re-read at `Turned` |
| `gallery/state.rs` memo | two entries (`state.rs:129-130`) | `with_details` | replaced by the details slots |
| `animation.rs` `restart` | `catalogs().file_path` | the viewer starting an animation | point lane |
| `video_source.rs` `play` | `catalogs().file_path` | the viewer starting a video | point lane |
| `yandex.rs` `search_file` | `catalogs().file_path` | the Yandex panel's search of a file | point lane |
| `library.rs` `load_roots` | `library().roots()` | `start`, `add_path`, `remove_folder` | the startup worker's slot at start ([Startup](#startup)); the command's ack after an add or a removal (R4); the point lane on a refresh of everything |
| `library.rs` `rescan` | the roots read (`crates/app/src/library.rs:327-329`) | `rescan` (`library.rs:240`), before it queues the scans | point lane, then the control lane queues the scans (R7) |
| `shell.rs` `view_names` | `views().all()` | the saved views' list, after each save and delete, and at startup (`shell.rs:48`) | the startup worker's slot at start; the command's ack after a save or delete (R4) |
| `shell.rs` `apply_view` | `views().view` | the views menu, called synchronously from QML | an in-memory lookup in the saved views the GUI holds (R4) |
| `controller.rs` `Core::new` | `views().listing()`, `views().view`, `prefs().get()` | `Shell`'s construction during QML load, before the first frame | read on the startup worker and taken from the slot |

**The listing's `arrive`.** `Grid::arrive` reloads the edited ids (`crates/client/src/grid.rs:282-292`) through the
synchronous `fresh_entry`, for the ids the GUI has edited while the list loaded. The design needs no such reload. The
listing's rows go to the stored layer under R2 with the listing's revision. An edit made while the list loaded was acked
while the read was in flight (from Part 3 the blocking reply is applied as an ack; from Part 4 it may also still be
pending, and is then drawn over the new rows). The acked entry is in the sparse map at its higher revision, so the older
listing's row for it is refused, and R2's notes ask for its details again.

### Catalog writes on the GUI thread → commands

| Site | Call | What it does today | Target |
| --- | --- | --- | --- |
| `gallery/edits.rs` `rotate` | `library().turn` | writes the `file_writes` turn row, then `wake_writer` (`crates/app/src/library.rs:176-180`, `:339`: the write-back's slot mutex and `tend`). The image write happens on the write-back thread | command (R6: turns add up); the command thread wakes the write-back after the commit |
| `gallery/edits.rs` `set_rating` | `edit().rate` | `set_rating` (and its write-back) | command (R6: last value) |
| `gallery/edits.rs` `set_border_marked` | `edit().set_border_mark` | writes `border_marks` | command (R6: last value) |
| `shell.rs` `save_view`, `delete_view` | `views().save`, `views().delete` | writes `views` | command |
| `shell.rs` `execute` (`SaveSession`) | `views().save_listing` | writes `session` | command |
| `library.rs` `add_path` | `library().add_folder` | normalize and `check_folder`, then `add_root`, then `Runner::scan` | pending from the click; the probes on a probe thread, then one command for `add_root`; the command thread queues the scan after the commit (R7) |
| `library.rs` `remove_folder` | `library().remove_folder` | a roots read in its own statement (`crates/app/src/library.rs:315-322`), then `remove_root` (one transaction, `crates/catalog/src/roots.rs:58-72`), then `Runner::folder_removed` with the root's path (the job's queue and busy mutexes, and a stop request) | one command, one transaction, without the outer read; the write returns the path, and the command thread calls `folder_removed` after the commit (R7) |

**What an add's probes do.** `add_path` (`library.rs:211-212`) calls `app::get().library().add_folder`
(`crates/app/src/library.rs:298`), which calls `normalize_path` (`crates/library/src/folders.rs:29-31`) and
`check_folder`. On Windows `check_ntfs` (`crates/scan/src/refusal.rs:39-66`) makes the volume lookup, the drive-type
query, a handle open, the reparse-point test, the final-path call and the file-system query
(`crates/platform/src/dir.rs:117` onward). All of that is file or volume I/O, on the GUI thread today, inside the
`with_catalog` closure, holding a pooled connection but no lock. It moves to a probe thread (R7).

### Not the catalog, but on the GUI thread → worker (Part 5)

| Site | What it does on the GUI thread | Target |
| --- | --- | --- |
| **Preferences**: `prefs().update` (`settings.rs`, `shell.rs` `prefs_effect`), and the Yandex save (`yandex.rs:357`, which reaches `prefs.update` in `crates/app/src/yandex.rs:550`) | writes the preferences file. `JsonStore::update` holds its lock across the serialize and the fsync (`crates/app/src/store.rs:51-55`), and `get` takes the same lock (`store.rs:46`), so a GUI `get()` waits for a save's fsync | `update` changes and serializes under the lock, then releases it. The file write goes to the preferences writer, a thread of its own that always writes the newest snapshot, so queued saves cannot land out of order. It drains and closes in `app::shutdown`, so a save queued at exit is not lost. A failed save posts its notice from there |
| **Logging**: every `pg_log` call | takes the ring lock (`crates/log/src/lib.rs:269`) and the logger's `writing` lock, and appends synchronously: it stats the file, checks that it exists, and opens it, for each line (`lib.rs:240-249`, `crates/log/src/file.rs:53-63`) | a channel to a log thread, flushed on exit, on panic, and before the fatal exit (`crates/desktop/src/lib.rs:236-239`). A crash can lose the last lines, which is accepted for the GUI's own calls. `Inbox::post`'s log line goes the same way |
| **The log panel**: `pg_log::subscribe()` (`log_model.rs:112-113`) | takes the logger's `ring` mutex and clones the kept records (`crates/log/src/lib.rs:299-305`) | the subscribe moves to the thread that already reads the file |
| **`picker.rs` `start_for`** | `yandex().start_folder()` stats the saved folder (`is_dir`) and takes the search's state lock (`crates/app/src/yandex.rs:520-527`); `picker.rs:53` canonicalizes (`crates/platform/src/picker.rs:44-50`) | a probe thread |
| **The non-native picker's folder listing** (`picker.rs:102`, `:170`, Linux only) | lists a directory | the pool |
| **`set_thumb_height`** (`crates/app/src/library.rs:252`) | takes a mutex and stops a running pass (`HeightCell::set`, `crates/library/src/runner.rs:57-76`, which wakes the job after releasing its own mutex; the stop is `request_stop`, `crates/jobs/src/control.rs:117-121`, which takes the `paused` mutex) | the control lane, which keeps two heights in order. The GUI keeps its own copy of the height, because the layout uses it |
| **`set_paused`, `is_paused`** | take the job's pause and queue mutexes (`runner.rs`, by symbol) | the control lane, which keeps a pause and an unpause in order; the state is pushed back |
| **`save_crops`** (`gallery/edits.rs:124`) | takes the job's queue mutex and wakes it (`runner.rs`, `save_crops`) | the control lane |
| **`wake_job()`** in `on_pass`'s `turned` branch (`library.rs:142-144`) | `request_stop` and `wake` (`runner.rs`, `wake` and `wake_up`) | the control lane |
| **`Runner::scan`** after `rescan` | the queue mutex and `request_stop` | the control lane. The one after an add, and `folder_removed` after a removal, run on the command thread after the commit (Part 1) |
| **`library().status()`** in `refresh_progress` and `refresh_writes` (`library.rs:166`, `:193`) | takes the write-back's slot mutex (`crates/app/src/library.rs:211-213`), and `tend` restarts a dead write-back thread under it (`:125`) | the library pump's thread reads the status when an event arrives and hands it to the Qt pass with the folded changes (Part 3). It is not a `Change`, which stays `Copy` and small. The GUI keeps the last status it was handed |
| **The Yandex search**: `search().start` (`yandex.rs:102`, from `search_file` and `search_command_line`), `search().save` (`yandex.rs:357`, `crates/app/src/yandex.rs:533-551`), `search().poll()` on every pump pass (`yandex.rs:263`) | `start` and `save` take the search's `shared` mutex, publish and spawn a thread (`app/src/yandex.rs:421-436`); `poll` takes the same mutex and the pictures' `take_dead` (`:485-489`). How long the search's workers hold `shared` is not checked | `start` and `save` are requests to the pool, and the search pushes its state, as the library's status is |
| **The animation player's join**: dropping a `Player` (`crates/render/src/player.rs:55-65`), which `restart` does on every change of an animated item (`animation.rs:54`) | joins the player's thread, which may be inside `std::fs::read`. The drop also closes the player's sink, and the sink's thread-safety rests on that close happening on the GUI thread (`frame_sink.rs:20-23`). The join also makes sure the old player's last frame comes before the new player starts on the same video sink (`animation.rs:54-68`) | the GUI thread closes the old player's sink, so none of its frames is shown any more, and then hands the player to the pool to join |
| **Video**: `video_player.rs:14-16`, `cpp/video_source.cpp:47-60` | `File::open` (`crates/platform/src/file.rs:164-166`, the Windows variant; the other is not checked), then that handle opened as a `QFile` (`_open_osfhandle`, `QFile::open(fd)`). `Player::start` (`animation.rs:68`) reads its file on its own thread (`player.rs:72`), so it is not a GUI call | the pool opens the file and hands the handle to the GUI, which closes it unused if the viewer has moved on (`MediaView.qml:57-64`) |

**Accepted, and staying on the GUI thread** (see the [Goal](#goal-and-accepted-exceptions)):

- *The clipboard.* `setText` (`notices.rs:441`, `cpp/clipboard.cpp:12-13`), which Qt requires there. Whether it can wait
  on another process is not checked.
- *The native picker's modal dialog* (`picker.rs:85`): the user waits on it, and its own event loop runs.
- *Startup, once, before the first frame*: the `gpu_thread` join after QML load (`crates/desktop/src/lib.rs:194-197`),
  the pipewire probe (`lib.rs:113`), `Config::from_env` (`lib.rs:137`), and the log and panic-log setup (`pg_log::init`,
  `lib.rs:142`; `panic_log::install`, `lib.rs:146`), which open files. `app::open` itself (`lib.rs:164`) is not
  accepted: Part 3 moves it to the startup worker.
- *Shutdown*: the joins of [The shutdown's steps](#the-shutdowns-steps), and `~Provider`'s wait
  (`cpp/providers.cpp:131-136`).
- *Env-gated, harness and debug-build work*: `PG_RECORD`'s file creation (`controller.rs:57`) and its synchronous write
  of the init record and of every controller update (`controller.rs:65-66`, `:76`, `:129`); the screenshot harness's
  writes; and the debug build's live reload, which lists a directory and watches it (`cpp/live_reload.cpp:64-68`).

## Measurements

Two passes, both on 2026-11-13. Each probe ran on a copy of the live catalog (`catalog.sqlite`, 230,812 files, 2 roots,
no pending files), and called the same functions the services call. The live catalog was never opened. The probes were
temporary and are deleted, so these numbers cannot be rerun from the repo. Two temporary source changes were made and
restored with `git checkout`: the import's flush was timed, and `wal_autocheckpoint = 0` was added to `SETTINGS` for one
experiment. The library job and the write-back were never started, so no image file was written. Debug build unless
marked release.

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

Only one stall of 100 ms or more was seen per run, at the first 1000-frame checkpoint. The later checkpoints (about 20
of the 21 in a 3,000-turn run, at roughly 7 frames per turn) were not timed separately. So the data show that this
checkpoint stalled, and the stall disappears without it. They do not show that every checkpoint stalls ([Open](#open)).
The typical turn took 0.16 to 0.22 ms (p50) in both settings.

### Import flush, release (second pass; 2,000 pending files, four batches of 500)

| Batch | Flush (ms) | Pairing inside the flush (ms) | Pairing share | SQL share (flush minus pairing, ms) |
| --- | --- | --- | --- | --- |
| 1 | 2,333.5 | 1,775.1 | 76% | 558.4 |
| 2 | 2,386.2 | 2,193.3 | 92% | 192.9 |
| 3 | 2,066.1 | 1,875.3 | 91% | 190.8 |
| 4 | 2,235.1 | 2,041.6 | 91% | 193.5 |

The import took 15.15 s, with 854 pairs and no failed file. With a rating loop during the import, the flushes were 2,035
to 2,467 ms and the pairing 1,821 to 2,142 ms. Pairing is in-memory work (`crates/dedup/src/pairer.rs`), so the pairing
share is matching time, not SQL time.

### Import flush, debug (second pass; 500 pending files, one batch)

Flush 12,363.4 ms; pairing 12,159.3 ms (98%). The import took 35.8 s.

### Rating during the import (second pass)

`set_rating` with write-back on, one call per loop iteration on a second connection. The loop slept 20 ms between calls.
The run time divided by the count is about 52 ms (debug) and 66 ms (release), but that average includes the time the
loop spent blocked on the import's lock (about 12 s of the debug run, the one 12.4 s flush, and about 9 s of the release
run, its four flushes). An unblocked iteration took about 30 ms: the call plus a 20 ms sleep at the Windows timer's
granularity.

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

The machine has 64 GB of RAM with 21.7 GB free, so reading 2.9 GB did not evict the OS cache. The first phase is
therefore not a cold-cache measurement, and the cold cost is not established ([Open](#open)). The `Reader` clears its
page cache after each job (`crates/app/src/reader.rs:183`), so a point lane built on the `Reader` would pay the
fresh-connection cost on each request. The point lane keeps its page cache, so the warm figures apply to it.

### What the numbers show

- Reads don't wait on a writer: the 200 reads during the 2 s hold had a maximum of 0.16 ms.
- A GUI write waits for the whole hold. A 2 s hold made the rating wait 2.09 s. A 7 s hold made it fail at 5.3 s, so the
  edit was lost. That is the 5 s busy timeout.
- Each import batch holds the write lock for its whole flush: 2.0 to 2.5 s per 500 files in release, and 12.4 s in
  debug. In debug, two ratings failed with "database is locked" after 5.3 s. In release, none failed; the longest wait
  was 2.35 s.
- The start-state read (`Core::new`) runs before the first frame. It is a read, so it does not wait on a writer.

## Open

Nothing in this plan is built: Parts 1 to 6 are designed, with their tests under Part 2 and [Tests](#tests). This
section is what is not settled, and what was checked.

### Measurements that gate a decision

- **Checkpoints, counted. This gates Part 2b.** Count the checkpoints in a 3,000-turn run, and time each one. The turn
  timings already show that none but the first cost 100 ms or more. Not known: whether the later ones ran, what each
  cost, and what the first one pays for.
- **How long the job's transactions hold the writer.** The scan batch (10,000 entries, or a whole larger folder, with no
  cap), the borders batch, the pairing tiers and `mark_missing`, and the preparation's marks run by another process are
  not measured (Part 1). `remove_root` is measured in debug only. They decide how long an edit can stay pending.
- **Startup with the `Loader`.** Today the QML load overlaps the GPU device's creation, and `gpu_thread` is joined after
  it (`crates/desktop/src/lib.rs:194-197`). With a trivial root, the heavy load runs later, with the placeholder
  showing. Whether startup gets slower, and by how much, is not measured ([Startup](#startup)).

### Costs not measured

- **The hand-off to the writer.** Each write now crosses to another thread and back. Its cost per transaction, and for
  about 250 catalog tests that each gain a file and a writer thread, is not measured (Part 1).
- **A snapshot around each read.** A `BEGIN`, one more row read and a `COMMIT` around a point read that takes 0.02 to
  0.12 ms today (Part 3).
- **"Refresh everything" on the GUI thread.** A full layout pass follows each listing; its cost sets how far apart
  refreshes must be (R3).
- **Batch matching** in the import (Part 2a). The pairing shares of the second pass also come from a deleted probe and
  are not re-checked.
- **Cold-cache reads.** Reading 2.9 GB did not evict the OS cache on this machine (64 GB of RAM, 21.7 GB free). Evicting
  it needs far more reads than a probe should do, or a reboot. Until then, the point lane's cost is the warm figure,
  about 0.02 to 0.05 ms for a `file_path` read.
- **Rescan's scans.** `rescan` was not measured with the scans run: with the library not started, it only reads the
  roots.
- **Startup and shutdown durations.** The `gpu_thread` join, the pipewire probe, the log setup, `app::open` and the
  start state's reads on the worker, and every wait in [The shutdown's steps](#the-shutdowns-steps). They decide whether
  the shutdown's bound is typical, and how long the placeholder shows. They need the app launched and closed with
  timing.
- **How long an ack takes** to reach the Qt thread, which the 200 ms before the pending mark rests on (R4).

### Chosen values, not measured

| Value | For | Where |
| --- | --- | --- |
| 200 ms | before the pending mark shows, and before the "saving" state shows | R4, [Closing the window](#closing-the-window) |
| 5 s | between refreshes of everything | R3 |
| 10 s | before the busy state shows | R5 |
| 10,000 | file commands in the pending set | R5 |
| 256 | file changes in one pump pass before it becomes a refresh of everything | Part 3 |
| 3 s | the wait at session end | [The operating system ending the session](#the-operating-system-ending-the-session) |
| 1 s | the watcher's poll | R3a |
| 4 s | the grace on the job's and the write-back's stops | [The writer](#the-writer) |
| 50 ms | the slice a waiting caller polls its stop in | [The writer](#the-writer) |

The 5 s deadlines of Parts 1 to 3 (on each command, and on the open) and the busy handler's 5 s when no call is current
are today's busy timeout, kept, not chosen.

### Not designed

- **Removing a folder in shorter transactions.** With one writer, the order of the removal's steps is the queue's order.
  Whether that makes a split safe against a scan or a re-add between the steps is not designed (R7).
- **The inbox's `Store` trait.** What the inbox does is decided: each step is its own service write, with the drain as
  its deadline (Part 1). How the trait (`Store`, `crates/app/src/inbox.rs`), which has the busy loop built in today, is
  reshaped for that is left to the implementation.

### Not traced or checked

- **A neighbour that changed during the import's matching.** Whether a pair with an already-indexed file that changed
  between the matching and the commit should be re-checked inside the write, and whether today's import has the same
  window (Part 2a).
- **What a refused scan leaves.** `begin_scan` refuses a folder that is no longer a root (R7). What the job has already
  recorded for that scan before it begins is not traced.
- **Session end.** Whether `commitDataRequest` reaches the app in practice, and how long the system allows the handler.
- **Two `Database` values on one file in one process.** The tests of R3a and of the open rest on the file locks being
  held per handle. That is so on Windows; it is not checked for the Linux build.
- **Sites.** How long the Yandex search's workers hold `shared`; whether the clipboard's `setText` can wait on another
  process; `File::open`'s non-Windows variant; whether the import pipeline delivers a batch in item order (Part 2a needs
  only the batch's own order, which it has).
- **Measurement notes.** The first pass's release batch total is lower than the second's and is not explained. The
  catalog copy had "no pending files", yet the import runs had 500 and 2,000; how those were made pending is not
  recorded.

### Verified against the code

- *The catalog and SQLite.*
  - No read path writes: none uses a temp table, `ANALYZE` or a lazy insert, and `Catalog::open` writes nothing once the
    file is in WAL mode. So `query_only` breaks no read.
  - No transaction outlives one flush, and nothing reads its own uncommitted writes across transactions, so each can be
    one `write` call.
  - The keeper is only the prepared connection held open, so the writer's connection can take its place.
  - The rebuild ends with `wal_checkpoint(TRUNCATE)` and a close (`rebuild.rs:119-120`), and needs the exclusive hold.
  - A connection's own commit does not move its `data_version`, and a passive checkpoint on a hooked connection is safe.
  - The thumbnail pass's only catalog writes are the failure ledger's.
  - An entry's `rotation` is `files.rotation` and its `pending_turns` is the queued `file_writes` value.
- *rusqlite and the workspace.*
  - `busy_handler` and `wal_hook` take closures at the locked revision (`src/busy.rs:55-57`,
    `src/hooks/mod.rs:389-391`), `total_changes()` is exposed (`src/lib.rs:1064`), and the busy handler is invoked for
    `BEGIN IMMEDIATE`.
  - `unsafe` is allowed in the workspace (`undocumented_unsafe_blocks = "warn"`, `Cargo.toml:65`), and `pg-catalog`
    holds no mutex a panic could poison.
  - No lock-order cycle among the write-back's slot, the job's queue and busy mutexes and the inbox's locks.
  - The inbox's `idle()` and `listing()` take different mutexes, and the inbox thread holds neither across SQL.
  - The write-back has the file id at `Turned`.
- *Qt and the desktop.*
  - A quit closes every top-level window and is dropped if one refuses (`qguiapplication.cpp:2119-2130`); at a session's
    real end Qt emits `aboutToQuit` and then quits (`qwindowscontext.cpp:1159-1175`).
  - The bridge objects are QML elements built through `Default`, so the app cannot be passed to them.
  - `pg_client::Grid` is outside the recorded controller, `gallery::post` takes any closure, and a `Loader` parents its
    item, so the screenshot harness still finds the objects.
  - `Player::start` reads its file on its own thread (`crates/render/src/player.rs:72`), and `requestImageResponse` runs
    on Qt's pixmap-reader thread (`qquickpixmapcache.cpp:1000-1003`).
  - There is a single window, no window geometry is persisted, and there is no undo and no multi-select edit.
