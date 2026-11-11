# Notifications: the notices and the log

How the app tells the user what happened, and what it records for a developer. The design and its stages are in
[NOTIFICATIONS_PLAN.md](NOTIFICATIONS_PLAN.md), and the fixes made after three reviews of stages S1 to S3 in
[NOTIFICATIONS_FIX_PLAN.md](NOTIFICATIONS_FIX_PLAN.md) (its section letters, A to G and W, are quoted below where a
rule comes from one). This is the reference for the code as it is: the notice types, the inbox and its writer, the
failure ledgers, the log, dev mode and the log view.

## Notices (`pg_model::notice`)

A `Notice` is a typed value, built by the crate that has the outcome and worded by `pg_client::text` for each front
end. Nothing in a notice is English, so it is read in the reader's language, and a stored one is worded again when the
words change. Each kind is one variant with:

- its **code**, the stored tag and the root of its words' ids (`notice-<code>-title`, `notice-<code>-body`). Codes are
  permanent: a stored row of a code this build doesn't know reads as `Notice::Unknown`, which is never written back.
  The codes come from one table, the `ids!` macro, which also gives the words' ids; `Notice::RETIRED` lists the codes
  of kinds that were removed (`scan-unreadable`, `mount-refused`, `root-refused`, now the folder ledger), which no kind
  may take again and whose rows read as `Unknown`.
- its **level** (info, warning, error) and its **source**, the inbox's category (`source-<code>`).
- its **lifecycle** (`Lifecycle`): an *event* is kept each time it is posted; a *transient* is a toast and a log record
  only (feedback on what the user just did); a *state* is kept under its key, replaced by the next post of that key and
  cleared once the problem is gone; a *ledger* entry is built from a failure ledger when the inbox is read.
- for a state, its **resolver** (`Resolver`): what clears it. `Lifecycle::State(key, resolver)` carries it, so a state
  without one doesn't compile. A state about a library folder is also cleared when the folder is removed
  (`root-removal`, through the catalog's foreign key). The table is below.
- its **actions** besides dismissing it: copy the details, retry, show the failed files.

A state's key is its kind, the library folder it is about (resolved to the folder of the library that holds it, so
removing the folder clears it) and a part, which is always a code (a scan note, an operation, a pass), never a folder:
per-folder facts are the catalog's folder ledger.

The stored form is JSON, internally tagged by `kind`, kebab-case; a field added later is `#[serde(default)]`.
`Notice::samples()` holds one notice of each kind. Each sample's stored form is pinned in `model/src/notice/golden.jsonl`
(the test checks that each writes its golden line and reads back as the sample, and names the kind that differs), and
the code is checked against the serde tag. The `ids!` table stays the one source of the codes: the tag is pinned by
`the_code_is_the_stored_kind` and the golden test, and a derive (strum) would be a third mechanism.

Text a notice carries is tidied (`notice::tidy`): control characters become spaces, and past its cap (`ERROR_CAP`, 2 KiB,
for an error) it is cut at a character's edge and ends in `…`, with the mark inside the cap, so the result is never
more than the cap's bytes. A path in the words is bidi-isolated (U+2068 … U+2069, `client/src/text/notice.rs`), so a
folder named in Arabic or Hebrew doesn't reorder the words around it; the bundle's own isolation stays off.

### Adding a kind

1. A variant of `Notice`, with its code in the `ids!` table, and its arms in `level`, `source` and `lifecycle`
   (exhaustive matches: it doesn't compile without them). A state names its resolver there.
2. A sample in `Notice::samples()`, its line in `notice/golden.jsonl`, and an arm in the test `ordinal`. A state's row
   in the resolver table below.
3. Its words: the `notice-<code>-title` and `-body` entries in `client/locales/en/text.ftl`, and its values in
   `client/src/text/notice.rs` (`args`). The parity test checks the file's entries against the samples' ids.
4. The call that posts it, from the crate that has the outcome, and the call that clears it, where its resolver is.

A removed kind's code moves to the `retired:` list of `ids!`. A new action is a variant of `Action`, the window's
handler for it, and the kinds that offer it in `Notice::actions`.

### What clears a kept notice

Every kept kind and part, and its resolvers, in the samples' order. The table is the output of
`Notice::resolver_markdown()` (from `Notice::resolver_table()`), copied here: change it with the code. The model's
test `the_documented_resolver_table_is_the_generated_one` fails, printing the table, when this copy differs.

| Kind | Part | Cleared by |
|---|---|---|
| `scan-note` | `duplicates` | `root-walk`, `root-removal` |
| `scan-note` | `reparse` | `root-walk`, `root-removal` |
| `scan-note` | `ambiguous` | `root-walk`, `root-removal` |
| `scan-note` | `unpaired` | `root-walk`, `root-removal` |
| `scan-note` | `volume-id` | `root-walk`, `root-removal` |
| `round-failed` | — | `next-success(round)` |
| `cache-full` | — | `complete-pass(thumbs)` |
| `prefs-not-saved` | — | `next-save` |
| `cache-touches-not-written` | — | `cache-recovery` |
| `cache-not-stored` | — | `cache-recovery` |
| `write-failed` | — | `status-poll` |
| `write-stopped` | — | `status-poll` |
| `listing-failed` | — | `next-success(listing)` |
| `row-read-failed` | — | `next-success(row-read)` |
| `reload-failed` | — | `next-success(reload)` |
| `facets-failed` | — | `next-success(facets)` |
| `tree-failed` | — | `next-success(tree)` |
| `views-failed` | `list` | `next-success(views-list)` |
| `views-failed` | `save` | `next-success(views-save)` |
| `views-failed` | `delete` | `next-success(views-delete)` |
| `session-not-saved` | — | `next-success(session)` |
| `edit-failed` | `rate` | `next-success(edit-rate)` |
| `edit-failed` | `rotate` | `next-success(edit-rotate)` |
| `edit-failed` | `mark-border` | `next-success(edit-mark-border)` |
| `unreadable-files` | `thumbs` | `complete-pass(thumbs)` |
| `unreadable-files` | `borders` | `complete-pass(borders)` |
| `unreadable-files` | `import` | `complete-pass(import)` |
| `ledger-not-written` | `thumbs` | `next-success(ledger-write-thumbs)` |
| `ledger-not-written` | `borders` | `next-success(ledger-write-borders)` |
| `ledger-not-written` | `import` | `next-success(ledger-write-import)` |

The table lists every part the type allows. Which are posted: `unreadable-files` by the thumbnails' and the borders'
passes (the import doesn't post it), and `ledger-not-written` by the thumbnails' pass only (below).

What each resolver is, in the code:

- `next-success(op)`: the next success of the same operation clears it (the window's reads and edits through
  `app::resolved`, the job's round, a ledger write).
- `complete-pass(pass)`: the next complete run of the pass, one that read every file it had to (the runner's
  `files_read`, and `cache-full` after a complete thumbnail pass that didn't fill the cache).
- `status-poll`: the writer's status as it changes. `write-failed` is posted by `pg_writeback::Writer` at its one status
  choke point when the error goes from none to some (or changes) and cleared when it goes back to none; `write-stopped`
  means "restarts exhausted", posted where `pg_app::Library::tend` gives the writer up (a stop at shutdown, or a panic a
  restart recovers from, posts nothing). `Library::start` clears both before the new writer starts, since a new writer
  starts with no error and would never see the change that clears an earlier run's state.
- `cache-recovery`: the thumbnail cache's writer publishes `TouchesWritten` or `Stored` at its first success after a
  failure; `pg_thumbcache::Event::effect` turns its events into a post or a clear, which the app's forwarder applies (the
  command line prints the posts only).
- `next-save`: the next save of the preferences (`PrefStore::update`).
- `root-walk`: the next complete walk of the whole library folder replaces the folder's kept scan notes with the ones
  it found.

## The inbox

The notices kept are the `notices` table of `catalog.sqlite` (migration `TO_3`, changed by `TO_8_NOTIFY`), read and
written by `pg_catalog::inbox`: a row holds the notice as JSON with its kind, level and source beside it, a count of its
posts and their first and last times, its read and dismissed marks, and its revision. A row is named by its id, its
`uid` (`meta.seq` when it was made; ids are reused) and its `rev`. `root` is the library folder it is about, a foreign
key that removes the row with the folder.

- An **event** is a row each time it is posted. Dismissing one deletes it (nothing can bring it back). Events are
  bounded on their own: past `kept_events!()` (1000) keyless rows, the `notices_uid` trigger deletes the oldest at each
  insert, by insertion order (`uid`), read ones first; never the row just inserted (so a row posted with an old time is
  never the one preferred), and never a state.
- A **state** is one row per key, bounded by its keys (codes × library folders × parts, all finite). Posted again it is
  counted and holds the latest; if it says something new (its data changed) its `rev` rises and it is unread again,
  even if it was dismissed, and if not it keeps its marks: a problem that holds is not news each time. A dismissed state
  is kept, so a repost of the same data stays dismissed, for 30 days (`DISMISSED_FOR`, `Catalog::prune_inbox`). Its
  resolver clears it (`inbox::clear`), or replaces the whole set of a kind and folder (`Catalog::replace_notices`: a
  complete walk of a library folder posts what it found and clears the rest).
- A notice about a folder no longer in the library is not kept.
- At most 200 entries are listed (`pg_app::inbox::LISTED`), rows and ledgers together, newest first.

**Marks address a revision.** A mark or a dismissal names `(id, uid, rev)`, so one sent for revision N never lands on
N+1 and swallows its news; a row since gone is a no-op. An entry's identity for the window (`EntryId::entry()`) leaves
the revision out, so a new revision is the same entry changed, not a new one. "Mark all read" and "Clear all" cover the
ids the window displayed (and the ledgers at the serial it displayed), never an entry it wasn't given. Every write
(`post_notice`, `replace_notices`, `clear_notices`, the marks, the dismissals, `record_failures`, `clear_failures`) says
whether it changed a row, and the writer reads the inbox again only after a real change or a `data_version` change.

Before a catalog of an earlier version is migrated, it is copied aside as `catalog.sqlite.pre-v<N>.bak` (once): the
migrations can't be undone, and an older build refuses a later catalog.

### The writer (`pg_app::Inbox`)

`pg_app::Inbox` (`App::inbox`) is the one way in. `post` logs the notice on the caller's thread (a kept state posted
again within `LOG_AGAIN`, a minute, is not logged again) and queues it for the inbox's writer, a thread with its own
connection. The writer keeps it, then reads the inbox and publishes what it lists (`InboxEvent::Changed`) and a `Toast`
for each entry that is news. What another process changed (the command line, a pass's own connection) is found by the
catalog's `data_version`, every 30 s (`POLL`), when the window is activated, after each of the job's rounds, and
whenever a pass changes a ledger (`Inbox::poke`). The window never reads the catalog for the inbox's listing.

**One work log (W1).** What is queued is one `Work`, under one lock with a condvar, shared by the handles and the
writer: a wake is never lost to a full channel, and a close never blocks. Every op takes a sequence number under that
lock, in the same critical section as its push, and the writer applies what it takes in that order across every kind
of op (a thumbnail failure reported before the user's Retry is written before it, so the Retry relists it). The lock is
held only to push or to swap, never across SQL. `Work` holds:

- Events: a bounded queue (`EVENTS`, 1024). One it has no room for is counted toward `NoticesLost`.
- States' posts, replaces and clears, which supersede the waiting ops they cover: a post replaces the waiting post of
  its key and carries its repeats forward (they sum even when the data differs, so the count never under-counts); a
  replace or a clear removes every waiting op inside its scope. 50 same-data posts between two drains are one upsert
  whose count rises by 50. Bounded by the distinct keys and scopes waiting.
- Marks, dismissals and the Retry: at human rate, never dropped.
- The window's thumbnail reports (`thumb_failed`, `thumb_made`), the latest per file.
- The flags: whether the inbox is viewed (the latest wins), a poke, the stop.

Whether a clear has work is decided by SQL on the writer: `replace_notices` reads `notices(kind, root)` first and
takes no write lock when nothing is stored.

**Failures (W2).** The writer's connection waits `BUSY_TIMEOUT` (100 ms, set by `Catalog::set_busy_timeout`; every other
connection waits 5 s) for a lock another connection holds, so it checks its stop at least that often. A write that is
still busy (`pg_catalog::Error::is_busy`, which walks the error's chain) is tried again, in order, until it goes
through: a scan's batch or an import's flush only delays it. Any other failure, `SQLITE_LOCKED` included (a conflict
inside the connection, which trying again on the same connection could loop on), is not tried again: a lost event or
state op counts toward `NoticesLost`, and a mark or a dismissal that fails is logged and the inbox is read again, which
undoes the window's optimistic change (it doesn't count as lost). `NoticesLost` is posted after the next write that goes
through, and counts lost notices only. The Retry is one writer command: it writes `Catalog::retry_failures`, then wakes
the job; a failure reads the inbox again. The writer applies its ops through a small `Store` trait, so the tests inject
`SQLITE_BUSY` and `SQLITE_LOCKED`.

**Shutdown.** `Inbox::close` sets the stop under the `Work` lock and wakes the writer, which goes on writing (and trying
a busy catalog again) what is queued until it is empty or `SHUTDOWN_DRAIN` (5 s) has passed, then logs what is left.
`App::shutdown` is idempotent, and `impl Drop for App` calls it, so a front end that only drops the app still closes the
cache, the inbox and the catalogs in their order.

**Counts only when viewed.** A ledger entry is listed from its cheap state (below). Its counts (the total, how many are
new, how many came since a dismissal) are read only while the inbox is open (`Inbox::set_viewing`); otherwise
`Entry.counts` is `None` and nothing renders a count. A row's count is its posts, always read.

**The toast rule.** An event toasts when it is new. A state toasts when it becomes unread (new, or posted again with new
data after it was read or dismissed), or when its `rev` rises while it is unread, at most once per entry per
`LOG_AGAIN` (60 s): a run of failing files toasts once, and the entry shows the latest. A ledger toasts when it turns
unread. A transient notice is only logged and toasted. The startup's listing is the baseline and toasts nothing.

**The decoder.** A stored row of a known kind whose payload doesn't read (a field it needs is missing) reads as
`Unknown` with the error, which the writer logs once per kind per process; a retired or unknown kind is expected and
not reported.

The job posts through `pg_jobs::NoticeSink` (`post`, `replace`, `clear`, `poke`), which the inbox implements; it lives in
`pg-jobs` so that the job (`pg_library::runner`) and the writer (`pg_writeback`) both post through it. Tests use
`pg_jobs_testkit::RecordingSink`. The command line passes a printing sink and prints the same words
(`pg_client::text::notice_line`) for the notices it has. A rebuilt catalog's notice is `Rebuilt::notice`.

## The failure ledgers

The files a pass failed on are the `file_failures` table of `catalog.sqlite` (migration `TO_4`, changed by
`TO_8_NOTIFY`), read and written by `pg_catalog::ledger`: one row per file and pass (thumbnails, borders, import),
holding the file as the pass read it (mtime, size, rotation), the error, whether the pass tries it again by itself
(`retry`), when it first failed, a serial (`meta.seq` when it was recorded) and whether it is hidden.

- **Thumbnails and borders** write their own rows. A failure is recorded only while the catalog has the file as the pass
  read it. Recorded again unchanged, nothing is written and the serial stays: a file that fails at every pass is not news
  each time. A new error updates the row and keeps the serial. A file whose mtime, size or rotation changes leaves these
  ledgers (`files_failures_stale`), as does a file made since (the pass's flush, a thumbnail the window made on demand)
  and a replaced file (`pair.rs`). A deleted file's rows go with it.
- **The import's rows mirror its failed analyses**, kept by triggers on `analysis` (`analysis_import`,
  `analysis_import_rewritten`, `analysis_import_removed`), with the state of the bytes the analysis read.
  `analysis_import` (version 9, in place of version 8's two) fires its body only for a failure or for a file that has
  a row, so the common analysis, a success with nothing recorded, costs the import one index probe. A re-analysis that fails on the same bytes (an `IMPORT_VERSION` bump, `gallery import --retry`) keeps
  the row and its serial; a changed file that fails again is a new row with a new serial (a dismissed entry comes back);
  a fixed file's row goes, and so does a deleted analysis's. An in-place update of the analysis by a write-back (the
  app rewrote the file: `file_rewritten`, `analysis_turned_back`) copies the new state into the row and keeps the
  serial. `files_failures_stale` doesn't touch the import's rows, and a Retry or a rotation that bumps `files.seq`
  doesn't either, so a thumbnail Retry never re-badges the import.
- **Missing is hidden, not removed.** A file gone missing keeps its rows of every pass, and the import's rows are also
  hidden while the file is pending (`hidden`, kept by `files_failures_hidden`, which fires only when `pending` or
  `missing` changes). They come back with their old serials, so an offline blip or a folder removed and added again is
  not news. The inbox reads the rest through the partial index `file_failures_live(pass, serial) WHERE hidden = 0`, and
  every read repeats that literal condition (`live_failures!()`), since SQLite uses a partial index only then. The inbox
  and the grid's failed filter agree by construction.
- What a failure is decides what follows (`pg_render::maker::MakeError`): a file that can't be read (`Unreadable`: an
  offline drive, a file in use or gone) is not recorded, only counted, and the pass tries it again; the job's
  `unreadable-files` state says how many, and clears after a complete pass with none. A file that doesn't decode
  (`Undecodable`) is recorded with `retry`, and the next pass tries it again. A thumbnail that panicked, and a border
  search that failed or panicked, are recorded without `retry`: the pass doesn't list them again until the file changes,
  or the user retries.
- **The thumbnails' pass** buffers its failures and writes them through its own connection in batches, or at least every
  2 s (`LEDGER_EVERY`), whatever ends the pass. A write that fails doesn't fail the pass (its files are tried again) and
  posts `ledger-not-written` (`LedgerOutcome::Failed`) until one works (`Written`; an empty pass is `Untouched` and clears
  nothing). This state is the thumbnails' only: their ledger is in the catalog and their results in the cache, two
  databases.
- **The borders' rows are written in the borders transaction** (`Tx::ledger_writer`, beside `Tx::borders_writer`): a
  batch stores its results and its ledger rows or neither, so a failure leaves the files due and is the round's
  failure (`round-failed`), not `ledger-not-written`.
- **The window's on-demand thumbnails** (`app/src/images.rs`): a thumbnail that doesn't decode is recorded through the
  inbox's writer (each file once while it is among the last 256), before the fallback to a smaller kept thumbnail, and
  logged once. An original that can't be read, a file being rewritten and a file the catalog doesn't have are not
  recorded. A thumbnail the window makes takes its file out of the ledger, through the writer too, only while the ledger
  has rows.

### An entry's state: shown, new, unread (A1, A4)

Each ledger is one entry of the inbox (`EntryId::Ledger { kind, seen }`, where `kind` is a `LedgerKind`: a pass's
files, or `scan-folders`). `ledger_ack` holds the user's marks by serial: `seen_to` (the newest serial listed when they
last read or dismissed it), `dismissed_to` (the newest serial listed when they dismissed it, or none) and `read`.
`Catalog::ledger_state` reads the ledger's newest and oldest live rows, one index seek each, and applies the rules:

- **shown** iff it has live rows and (`dismissed_to` is none, or a row has `serial > dismissed_to`);
- **new** iff a row has `serial > seen_to`;
- **unread** iff shown and (never read, or new).

Marking it read moves only `seen_to`; dismissing it moves `dismissed_to` and `seen_to`; neither ever moves back. So
opening the inbox never hides an entry, and a dismissed entry comes back with its next row, saying how many came since
it was dismissed. The counts (`Catalog::ledger_counts`) are read only while the inbox is open.

**`THUMBS_VERSION`** (`pg_render`) is the thumbnailer's version, bumped by hand when a change of the decoder or the maker
could make a file that failed work. The catalog keeps the version it last ran under (`meta.thumbs_version`): a new one
relists the thumbnails that won't be tried again (`retry = 0`) and empties the thumbnails' ledger. The cache keeps its
own mark, and a new one makes every height forget how far it was checked, so the next pass looks at every file once. A
new `BORDERS_VERSION` empties the borders' ledger, as the pass redoes them all.

**Retry** (the thumbnails' entry) is `Catalog::retry_failures`: the rows go, and the files are listed again by the next
pass, which the job is woken for. The import's rows mirror its analyses and are never deleted by a Retry; its failures
are retried by `gallery import --retry`, and its entry has "Show the failed files", which lists every file whose analysis
failed (the `failed` status, in every folder).

### The folder ledger (B2)

The folders a walk couldn't read are the `folder_failures` table: one row per folder, by its exact path (`UNIQUE(path)`,
as `folders.path`), tied to the library folder that holds it (a foreign key: removing the library folder, or `add_root`
replacing the folders inside a new one, removes its rows), with the reason (`unreadable`, `mount-refused`, `refused`),
the error, the `scans` row of the walk that last saw it (`walk`), a serial and when it was first seen. It is the inbox's
`scan-folders-failed` entry, acknowledged as `scan-folders` in `ledger_ack` with the same rules; its detail lists the
folders grouped by library folder, with the reason and the error, Copy includes them, and Reveal opens a folder's parent.

The rule is **written as it goes, swept at the end**:

- **During the walk**, each folder it can't open or list, and each refused mount, is upserted inside the batch's
  transaction that commits around it (`ScanWriter`'s `folder_failed`), with `walk` this walk's id. The covering library
  folder is resolved in Rust once per walk (`types::covering_root`, since the Windows rules compare names
  Unicode-lowercased, which SQLite's `lower()` doesn't); the insert checks the library folder's id and path, so a walk
  of a folder no library folder covers, or one whose library folder was removed mid-walk, writes nothing and doesn't
  fail. An unchanged unreadable folder keeps its serial and `first_seen`: it is not news again.
- **At the end of a complete walk of folder F**, in `mark_missing`'s last transaction, the rows under F whose `walk` is
  older than this walk's go, except those strictly below a folder the walk skipped as unreadable or a refused mount: it
  never looked there, so it can't say they resolved. A reparse folder spares nothing. A root walk sweeps its library
  folder; a sub-folder walk sweeps exactly its subtree, so nothing is counted twice and nothing outside the walk is
  touched. `scans` ids only grow, and the scan lease means no overlapping walk runs meanwhile.
- **An incomplete walk sweeps nothing**: what it wrote is still true.
- **A refused root** (the walked folder itself: not NTFS, a FAT drive) is refused before any walk, so there is no scans
  row; `Catalog::folder_refused` writes it in a small transaction of its own with `walk` the newest `scans` id, so the
  next complete walk of it or of a folder above it sweeps it.

The scan's other notes are split by what they describe (`ScanNote::lifecycle`): the conditions of a library folder
(`duplicates`, `reparse`, `ambiguous`, `unpaired`, `volume-id`) are states keyed `(root, code)` and replaced only by a
root walk; what one walk did (`replaced`, `rebound`) is an event of that walk, with its count.

## The scan and the write lock (W2)

A walk never holds the catalog's write lock across the file system or a wait (`pg_scan`):

- Each folder is listed outside any transaction, into the batch being built, with the catalog's `(size, mtime)` of its
  files as they were then (`Catalog::prior_states`). A batch is written at a folder boundary once it holds `BATCH`
  (10 000) entries, in one short write transaction (folder rows, the files seen, the new files, the folder ledger's
  rows), and committed at once. A folder larger than a batch is held whole.
- The batch changes a file's row only where it still has the size and mtime that were read and no write of the app's
  into it is in progress (`not_being_written!`): a write-back committed between the listing and the batch is kept, and
  the file is only marked seen; the next walk lists its new state. Stamps such as `files.seq` are not compared, since
  imports, Retry and rotations bump them.
- A pause or a stop first writes the batch built, then waits with no transaction open. `mark_missing` still runs only at
  the end of a complete walk. Pairing (`pair_after_scan`) and `mark_missing` run in batches of at most 1000 rows, each in
  its own transaction.
- **Two walks of overlapping folders never interleave** (one is inside the other): a walk holds a **scan lease**
  (`scan_leases`, `Catalog::begin_scan`, which refuses when an unexpired lease covers an overlapping folder). The walk
  renews it with each batch and every 20 s while it is held up; one with no heartbeat for `SCAN_LEASE_SECS` (60 s) is a
  crashed process's, and expires. It is given back in `mark_missing`'s last transaction, or when the `ScanLease` is
  dropped (a stop, an error, a panic). A refused walk is `ScanError::Busy`: the job puts it back on its queue and tries
  again after 5 s (`BUSY_RETRY`); `gallery scan` says another process is scanning and fails, or waits with `--wait`.

So the walk's write lock is held for one batch write's milliseconds, and a paused scan holds nothing. The inbox's writer,
ratings, rotations, border marks, write-back and the command line no longer time out behind a paused scan.

## The window

`Notices` (`src/notices.rs`): the status bar's Inbox button shows the unread count and opens the inbox; the newest toast
is in the status bar (an info goes after 6 s, a warning stays until it is replaced or the inbox opens). The inbox
(`InboxPanel.qml`) lists the entries, newest first. The list is keyed by entry and updated by a diff of each listing
(`list_diff.rs`), so a new notice keeps the selection and the open details, and it is driven from the keyboard (the keys
are the fix plan's D1). Opening the inbox marks read the entries it displays, and entries arriving while it is open and
the window active are marked when they are shown. A row's details have Copy (to the clipboard) and Dismiss; Clear all
dismisses the entries displayed. **Ctrl+Shift+N** opens and closes it. `--inbox=1` opens it for the screenshots, and
`--inbox-expand=<kind>` opens a ledger's entry.

**The status bar's link (C1).** The job line says "N failed" as a link only while the inbox lists that ledger (a
`passes` property with a change signal, a bit per ledger with a listed entry or an `unreadable-files` state); otherwise it
is plain text. A click opens the inbox expanded at that entry, and an entry that appears later expands when it does. A
pass that writes or clears ledger rows calls `Control::ledger_changed`, whose hook the runner sets to the inbox's poke
(the thumbnails' and the borders' passes after each write or clear that changed rows, the import after every batch,
since its rows move by trigger, and the scan); pokes coalesce in `Work`. The thumbnails' pass flushes at least every
2 s, so the entry follows a failing file within about 2 s.

**The inbox's own reads leave the GUI thread (C2).** A ledger entry's list (its newest files, or its folders, with the
counts read together), what Copy adds to an entry's words, and the folder a Reveal opens are read by the window's
`Reader` (`pg_app::reader`, the kinds `FailedFiles`, `CopyDetails` and `RevealFolder`), which keeps only the newest
request of each kind per session and cancels a stale one; each kind is its own, so none cancels another. Each caller
asks, then goes on when its read is delivered: the list shows "loading", Copy copies, Reveal lists the folder or toasts
`FileNoLongerInLibrary`. These reads are the user's, so waiting behind a listing that is running is acceptable. The
synchronous `Catalogs::failed_files` and `Catalogs::present_folder` are gone. The rule: what the notifications added or
use reads nothing on the GUI thread. The rest of the window's catalog work on that thread was not added by this work and
is its own plan: [GUI_THREAD_PLAN.md](GUI_THREAD_PLAN.md).

**Reveal waits for its own listing (C3).** The controller stamps each `Command::OpenList` with a generation from one
counter of its own; the grid carries it (through `GalleryModel::open_listing` and `Grid::arrive`, never through QML),
and `Shell::list_shown` reports the generation shown. A reveal is consumed only by its own listing's `ListShown`, and
navigating away drops it.

## The log (`pg-log`)

`gallery.log` in the data folder, written by the window only (the command line prints to stderr). Each record is one
JSON line:

| Field | What it holds |
| --- | --- |
| `t` | Milliseconds since 1970, UTC (absent on an older build's plain line) |
| `local` | The local time it was written at, `2026-11-10 17:04:16`: what the log view shows |
| `level` | `info`, `warning` or `error` (the notices' levels) |
| `target` | What it is about, a word: `session`, `library`, `gallery`, `panic`, ... |
| `msg` | The message, at most 4 KiB (`MESSAGE_CAP`) |
| `detail` | Optional: a backtrace, a notice's details; at most 16 KiB (`DETAIL_CAP`) |
| `pid`, `session`, `seq` | The process, when it started, and the record's number in it: a record's identity |
| `file_only` | Present (true) only on a record written to the file alone (below) |

`DETAIL_CAP` is 16 KiB, not the message's 4: a panic's backtrace is a detail, one passes 4 KiB (a few dozen frames, each
with its file and line), and the frames that place the panic are its end. A capped text ends in `…`, the mark inside
the cap.

Each run starts with a `session` record whose detail is the build and the data folder. A record is appended as it
comes, in one write, by a file opened and closed for it, so a reader or a second instance sees every line, and a
panic's write needs no lock (one `write_all` of one line on an append handle is one append). Before the log has its
file, and when the file can't take a record, the record goes to stderr too.

**Rotation never sleeps.** Past 5 MiB (`FILE_CAP`) the file is rotated to `.1`, `.1` to `.2`, the oldest dropped:

- the file is first renamed aside (`gallery.log.next`); only when that worked is `.1` shifted to `.2` and the aside file
  made `.1`, so a failed rename shifts nothing and a retry can't overwrite an older file;
- rotations are serialized between processes by an OS lock on `gallery.log.lock` (std's `File::try_lock`, released by
  the OS when a process ends): a process that finds it taken skips the rotation for this record;
- an aside file left by a crash between the renames is finished first, under the lock; the reader reads it meanwhile, in
  its place between `.1` and the file;
- **skip, then fall back:** when the file can't be renamed (another process has it open without sharing its deletion),
  the rotation is skipped for this record and tried at the next, so the file runs a few lines past its cap. Only once it
  has grown past twice the cap (the rename fails persistently) is it copied aside and emptied in place. That can lose a
  line another instance of the app appends between the copy and the emptying: **the one documented loss**.

The reader takes the plain lines an older build wrote (`2026-10-30 17:04:16 text`, as info, with their time), the lines a
plain record ran on to (a panic's backtrace, as its detail), and a JSON line the app was stopped in the middle of (as
text).

The newest records of this run (at most 5000, and 4 MiB of their text) are kept in memory from the process's start,
whether or not the log view is ever opened. A subscriber (`pg_log::subscribe`) gets them and then each new one; with the
files' records (`pg_log::read`) merged by `Feed::merge`, none is missed and none is there twice. Every hold of the
records in memory goes through one guard (`RingGuard`: keeping a record, the panic path, subscribing), which marks the
thread as holding them, so ordinary contention just waits and no record is lost. A panic raised while its own thread
holds them (which could not take them again without deadlocking) is written to the file only, marked `file_only`, and
`Feed::merge` keeps such records from the file, so the view still shows it once.

A panic is recorded by the panic hook (`pg_log::panic`), even when it came while the log was being written.

## Dev mode and the log view

Dev mode shows the log view's button in the status bar. **Ctrl+Shift+D** turns it on and off and saves it in the
preferences (`dev_mode`); `--dev=1` or `PG_DEV=1` turns it on for one run without saving it, and leaves the preferences
file as it was. **Ctrl+Shift+L** opens and closes the log view in dev mode, and **Ctrl+Shift+N** the inbox.

The shortcuts are Qt `Shortcut`s (`GlobalShortcuts.qml`), which Qt matches before the focused item's keys, so they work
from the grid, the viewer and a text field. What a shortcut does is the controller's (`pg_client::controller`): while a
dialog or a menu is open, it is that panel's and the shortcut does nothing; the log view's key closes it when it is open,
and opens it otherwise (closing the inbox first, and the inbox's the log). Each modal panel reports its opening and
closing (`Shell.panelOpened`, `panelClosed`, by its `PanelKind.qml` code), so the controller knows which are open. The
open panels are an ordered set (`Panels.modals`): opening a kind already open moves it to the top, and closing removes it
wherever it is, so it holds each kind at most once.

The log view (`LogPanel.qml`, over `LogModel`) shows this run's records and the files', oldest first. It follows new
ones while it is at the end, and stops following when the user scrolls away; the selection is by record (`seq`), kept
when the oldest rows are trimmed, and cleared if its own row was. A row is one line; a click shows its whole message and
detail below. Escape or Close dismisses it, and Tab stays in it. `--log-view=<file>` shows that file's records instead,
read with the `pg_log` reader and subscribed to nothing (the `log` screenshot); the app's own logging is unchanged.

## Plain text

A `Label` or `Text` shows a value that looks like markup as rich text (Qt's `AutoText`), so a name, a path or an error
holding `<b>` would be drawn as HTML. Every one whose text isn't fixed words sets `textFormat: Text.PlainText`; the test
`qml_text::every_label_that_shows_a_value_is_plain_text` (`crates/desktop/src/qml_text.rs`) fails on one that doesn't.

## Storage, as `TO_8_NOTIFY` leaves it

The inbox's and the ledgers' layout is in `crates/catalog/src/schema.rs` (`TO_3_NOTICES`, `TO_4_FAILURES`,
`TO_8_NOTIFY`). The last added:

- `ledger_ack.dismissed_to` (a serial), replacing the `dismissed` time;
- `notices.rev`, the index `notices(kind, root)`, and `notices_uid` recreated with the Events' bound;
- `file_failures.hidden`, the partial index `file_failures_live`, `files_failures_stale` narrowed (no import rows, and
  no longer on `missing`), and the triggers `files_failures_hidden` and `analysis_import_*`;
- `folder_failures` with its serial stamp, and `scan_leases`, which a rebuild doesn't carry (`rebuild::TRANSIENT`).

A migration is a list of steps (`schema::Step`): SQL that changes the layout, or a Rust conversion of rows
(`crate::convert`), which finds the old shape by the columns it has, never by the version, and is idempotent. The
rebuild's copy runs every `Rust` step of every migration on the copied rows, so a catalog rebuilt from any version is
converted as its migrations would have converted it. `convert_notify` deletes the retired scan notices' rows, makes
`write-dropped` rows events, backfills an import row for every failed analysis (`ON CONFLICT DO NOTHING`, serials in
`(files.seq, file_id)` order), computes `hidden`, and, once, from the old shape, turns each dismissal into the serial it
was made at (none for an entry read after it was dismissed, which came back) and the import entry's old `files.seq`
marks into the backfilled serials they covered (or the serial before them all: "seen everything before the upgrade").

## Where the window tells

Every report of the window, and what it is: the notice it posts (the inbox logs it, its target the notice's source),
or a log record when it stays a diagnostic. A state is cleared by its resolver (the table above).

| Site | Target, level (logged before S2) | What | Now |
| --- | --- | --- | --- |
| `lib.rs` `run` | `session`, info | The run's start: the build and the data folder | log |
| `lib.rs` `run` | `config`, warning | `PG_THUMB_CAP_GB` didn't parse | log |
| `panic_log.rs` hook | `panic`, error | A panic, with its backtrace | log (the box, for a fatal one) |
| `app.rs` `stop` | `app`, warning | The background work didn't stop | log |
| `controller.rs` `Core` (3) | `record`, warning | `PG_RECORD` couldn't be written | log |
| `pg_app` `yandex::Yandex::new` (from `--yandex-fixture`) | `yandex`, info | A recording answers the searches | log |
| `pg_app` `App::stop` | `yandex`, warning | A search, a save or a fetch still ran at the stop | log |
| `gallery/state.rs` `with_details` | `gallery`, warning | A file's row couldn't be read | `RowReadFailed` |
| `gallery/state.rs` `fresh_entry` | `gallery`, warning | A file's row couldn't be read again | `ReloadFailed` |
| `gallery/model.rs` `open_listing` | `gallery`, warning | The list couldn't be read | `ListingFailed` |
| `gallery/edits.rs` `rotate`, `set_rating`, `set_border_marked` | `edit`, warning | An edit couldn't be saved | `EditFailed` |
| `filters.rs` `count_facets` | `filters`, warning | The filter bar's counts couldn't be read | `FacetsFailed` |
| `folders.rs` `reload` | `folders`, warning | The folder tree couldn't be read | `TreeFailed` |
| `shell.rs` `view_names`, `save_view`, `delete_view` | `views`, warning | The saved views couldn't be listed, saved, deleted | `ViewsFailed` |
| `shell.rs` `execute` (`SaveSession`) | `session`, warning | The session couldn't be saved | `SessionNotSaved` |
| `picker.rs` `pick` | `picker`, warning | The folder dialog failed (the call, the start folder) | `PickerFailed` |
| `picker.rs` `report_unlisted` | `picker`, warning | The app's folder dialog couldn't list a folder | `PickerFailed` |
| `library.rs` `load_roots` | `library`, warning | The library's folders couldn't be read | `FoldersUnreadable` |
| `library.rs` `start` | `library`, error | The background work didn't start | `LibraryUnavailable` |
| `library.rs` `start` | `library`, warning | The change pump didn't start | log |
| `providers.rs` (4) | `providers`, warning | An image request failed, or wasn't one | log (a thumbnail that doesn't decode is also recorded in the ledger, by `pg_app::Images`) |
| `animation.rs` `restart` | `animation`, warning | An animation couldn't be read | log |
| `video_source.rs` `play`, `video_player.rs` `play_path` | `video`, warning | A video couldn't be opened | log |
| `yandex.rs` (3) | `yandex`, warning | A search's file, the pump, a thread | log (the panel says it) |
| `log_model.rs` `start` | `log`, warning | The log view's reader didn't start | log |

The writer's failures (`write-failed`, `write-stopped`) and a dropped edit (`write-dropped`, an event: a lost edit is a
fact, not a condition) are posted by `pg_writeback` and `pg_app::Library`, no longer by the window's `refresh_writes`,
which only shows the writer's status.
