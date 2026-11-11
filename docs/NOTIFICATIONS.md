# Notifications: the notices and the log

How the app tells the user what happened, and what it records for a developer. The design and its stages are in
[NOTIFICATIONS_PLAN.md](NOTIFICATIONS_PLAN.md); this is the reference for the code as it is. Stages S1 and S2 are in: the
notice types, the log, dev mode and the log view, and the inbox. The failure ledger (S3) comes next.

## Notices (`pg_model::notice`)

A `Notice` is a typed value, built by the crate that has the outcome and worded by `pg_client::text` for each front
end. Nothing in a notice is English, so it is read in the reader's language, and a stored one is worded again when the
words change. Each kind is one variant with:

- its **code**, the stored tag and the root of its words' ids (`notice-<code>-title`, `notice-<code>-body`). Codes are
  permanent: a stored row of a code this build doesn't know reads as `Notice::Unknown`, which is never written back.
- its **level** (info, warning, error) and its **source**, the inbox's category (`source-<code>`).
- its **lifecycle**: an *event* is kept each time it is posted; a *transient* is a toast and a log record only (feedback
  on what the user just did); a *state* is kept under its key, replaced by the next post of that key and cleared by its
  producer once the problem is gone; a *ledger* entry is built from the failure ledger when the inbox is read.
- its **actions** besides dismissing it: copy the details, retry, show the failed files.

A state's key is its kind, the library folder it is about (resolved to the folder of the library that holds it, so
removing the folder clears it) and a part (a refused mount, a scan note, an operation).

The stored form is JSON, internally tagged by `kind`, kebab-case; a field added later is `#[serde(default)]`.
`Notice::samples()` holds one notice of each kind, and the tests check that each reads back as written, that the code is
the stored tag, and the stored form of the kinds that cover each shape.

### Adding a kind

1. A variant of `Notice`, with its code in the `ids!` table, and its arms in `level`, `source` and `lifecycle`
   (exhaustive matches: it doesn't compile without them).
2. A sample in `Notice::samples()` and an arm in the test `ordinal`.
3. Its words: the `notice-<code>-title` and `-body` entries in `client/locales/en/text.ftl`, and its values in
   `client/src/text/notice.rs` (`args`). The parity test checks the file's entries against the samples' ids.
4. The call that posts it, from the crate that has the outcome.

A new action is a variant of `Action`, the window's handler for it, and the kinds that offer it in `Notice::actions`.

## The inbox

The notices kept are the `notices` table of `catalog.sqlite` (migration `TO_3`), read and written by
`pg_catalog::inbox`: a row holds the notice as JSON with its kind, level and source beside it, a count of its posts and
their first and last times, and its read and dismissed marks. A row is named by its id and its `uid` (`meta.seq` when
it was made; ids are reused). `root` is the library folder it is about, a foreign key that removes the row with the
folder. At most 1000 rows are listed; dismissed rows go after 30 days.

- An **event** is a row each time it is posted.
- A **state** is one row per key. Posted again it is counted and holds the latest; if it says something new (its data
  changed) it is unread again, even if it was dismissed, and if not it keeps its marks: a problem that holds is not news
  each time. Its producer clears it once the problem is gone (`inbox::clear`), or replaces the whole set of a kind and
  folder (`Catalog::replace_notices`: a complete scan of a library folder posts what it found and clears the rest).
- A notice about a folder no longer in the library is not kept.

Before a catalog of an earlier version is migrated, it is copied aside as `catalog.sqlite.pre-v<N>.bak` (once): the
migrations can't be undone, and an older build refuses a later catalog.

`pg_app::Inbox` (`App::inbox`) is the one way in. `post` logs the notice on the caller's thread (a kept state posted again
within a minute is not logged again) and queues it for the inbox's writer, a thread with its own connection. The writer
keeps it, then reads the inbox and publishes what it lists (`InboxEvent::Changed`) and a `Toast` for each entry that is
news: new, or unread again. What another process changed (the command line, a pass's own connection) is found by the
catalog's `data_version`, every 30 s, when the window is activated and after each of the job's rounds (`Inbox::poke`).
The window never reads the catalog for the inbox. A transient notice is only logged and toasted. A post the queue has
no room for is counted, and `NoticesLost` says how many. A clear of a kind the inbox holds none of sends nothing, so
the window can clear a kind at every read that works.

The job posts through `pg_library::runner::NoticeSink`, which the inbox implements; the cache's events become notices by
`pg_thumbcache::Event::into_notice`, a rebuilt catalog by `Rebuilt::notice`. The command line prints the same words
(`pg_client::text::notice_line`) for the notices it has.

In the window (`Notices`, `src/notices.rs`): the status bar's Inbox button shows the unread count and opens the inbox,
which marks every entry read; the newest toast is in the status bar (an info goes after 6 s, a warning stays until it
is replaced or the inbox opens). The inbox (`InboxPanel.qml`) lists the entries, newest first; a click shows a row's
details, with Copy (to the clipboard) and Dismiss; Clear all dismisses every entry. **Ctrl+Shift+N** opens and closes
it. `--inbox=1` opens it for the screenshots.

## The failure ledger

The files a pass failed on are the `file_failures` table of `catalog.sqlite` (migration `TO_4`), read and written by
`pg_catalog::ledger`: one row per file and pass (thumbnails, borders), holding the file as the pass read it (mtime,
size, rotation), the error, whether the pass tries it again by itself (`retry`), when it first failed and a serial
(`meta.seq` when it was recorded). The import's failures are not copied: its ledger is its failed analyses
(`analysis_failed`), with the file's `seq` as the serial.

- A failure is recorded only while the catalog has the file as the pass read it. Recorded again unchanged, nothing is
  written and the serial stays: a file that fails at every pass is not news each time. A new error updates the row and
  keeps the serial.
- A file whose mtime, size, rotation or missing mark changes leaves the ledger (the trigger `files_failures_stale`), as
  does a file made since (the pass's flush, a thumbnail the window made on demand) and a replaced file (`pair.rs`). A
  deleted file's rows go with it.
- What a failure is decides what follows (`pg_render::maker::MakeError`): a file that can't be read (`Unreadable`: an
  offline drive, a file in use or gone) is not recorded, only counted, and the pass tries it again; the job's
  `UnreadableFiles` state says how many, and clears after a complete pass with none. A file that doesn't decode
  (`Undecodable`) is recorded with `retry`, and the next pass tries it again. A thumbnail that panicked, and a border
  search that failed or panicked, are recorded without `retry`: the pass doesn't list them again until the file
  changes, or the user retries.
- The pass buffers its failures and writes them in batches (or every 2 s) through its own connection, whatever ends the
  pass; a write that fails doesn't fail the pass (its files are tried again) and posts `LedgerNotWritten` until one
  works.

Each pass's ledger is one entry of the inbox, beside the kept notices (`pg_catalog::inbox::EntryId::Ledger`), with its
count and how many are new. `ledger_ack` holds its marks: `seen_to` (the newest serial the user saw), read and
dismissed. It is shown while it has failures and isn't dismissed, or has failures newer than `seen_to`; it is unread
while it has never been read, or has new ones. A mark never moves `seen_to` back. The window's on-demand thumbnail
failures go to the inbox's writer (each file once while it is among the last 256), and a thumbnail it makes takes its
file out, through the writer too, only while the ledger has rows.

**`THUMBS_VERSION`** (`pg_render`) is the thumbnailer's version, bumped by hand when a change of the decoder or the
maker could make a file that failed work. The catalog keeps the version it last ran under (`meta.thumbs_version`): a
new one relists the thumbnails that won't be tried again (`retry = 0`) and empties the thumbnails' ledger. The cache
keeps its own mark, and a new one makes every height forget how far it was checked, so the next pass looks at every
file once. A new `BORDERS_VERSION` empties the borders' ledger, as the pass redoes them all.

In the window: the status bar's job line says "N failed" as a link while a pass has failures (`Library.failed`,
`Library.failedPass`); a click opens the inbox at that pass's entry. A ledger entry's details are its newest 200 files
(`FailedFiles`, `src/failed_files.rs`), each with its path, error and Reveal: the file's folder is listed with nothing
filtered, and the file selected once it is shown (or a toast says the library no longer has it). The thumbnails'
entry has Retry (`Catalog::retry_failures`: the rows go, and the files are listed again by the next pass, which the job is woken
for); the import's has "Show the failed files", which lists every file whose analysis failed (the `failed` status, in
every folder). Copy puts the entry and its files on the clipboard.

## The log (`pg-log`)

`gallery.log` in the data folder, written by the window only (the command line prints to stderr). Each record is one
JSON line:

| Field | What it holds |
| --- | --- |
| `t` | Milliseconds since 1970, UTC (absent on an older build's plain line) |
| `local` | The local time it was written at, `2026-11-10 17:04:16`: what the log view shows |
| `level` | `info`, `warning` or `error` (the notices' levels) |
| `target` | What it is about, a word: `session`, `library`, `gallery`, `panic`, ... |
| `msg` | The message, at most 4 KiB |
| `detail` | Optional: a backtrace, a notice's details; at most 16 KiB |
| `pid`, `session`, `seq` | The process, when it started, and the record's number in it: a record's identity |

Each run starts with a `session` record whose detail is the build and the data folder. A record is appended as it comes,
in one write, by a file opened and closed for it, so a reader or a second instance sees every line. Past 5 MiB the file is
rotated (`gallery.log` to `.1` to `.2`, the oldest dropped); a file that can't be renamed is emptied, so the log never
grows without bound. Before the log has its file, and when the file can't take a record, the record goes to stderr too.

The reader takes the plain lines an older build wrote (`2026-10-30 17:04:16 text`, as info, with their time), the lines a
plain record ran on to (a panic's backtrace, as its detail), and a JSON line the app was stopped in the middle of (as
text).

The newest records of this run (at most 5000, and 4 MiB of their text) are kept in memory from the process's start,
whether or not the log view is ever opened. A subscriber (`pg_log::subscribe`) gets them and then each new one; with the
files' records (`pg_log::read`) merged by `Feed::merge`, none is missed and none is there twice.

A panic is recorded by the panic hook (`pg_log::panic`), even when it came while the log was being written.

## Dev mode and the log view

Dev mode shows the log view's button in the status bar. **Ctrl+Shift+D** turns it on and off and saves it in the
preferences (`dev_mode`); `--dev=1` or `PG_DEV=1` turns it on for one run without saving it. **Ctrl+Shift+L** opens and
closes the log view in dev mode, and **Ctrl+Shift+N** the inbox.

The shortcuts are Qt `Shortcut`s (`GlobalShortcuts.qml`), which Qt matches before the focused item's keys, so they work
from the grid, the viewer and a text field. What a shortcut does is the controller's (`pg_client::controller`): while a
dialog or a menu is open, it is that panel's and the shortcut does nothing; the log view's key closes it when it is open,
and opens it otherwise (closing the inbox first, and the inbox's the log). Each modal panel reports its opening and closing
(`Shell.panelOpened`, `panelClosed`, by its `PanelKind.qml` code), so the controller knows which are open.

The log view (`LogPanel.qml`, over `LogModel`) shows this run's records and the files', oldest first, following the new
ones. A row is one line; a click shows its whole message and detail below. Escape or Close dismisses it, and Tab stays
in it.

## Plain text

A `Label` or `Text` shows a value that looks like markup as rich text (Qt's `AutoText`), so a name, a path or an error
holding `<b>` would be drawn as HTML. Every one whose text isn't fixed words sets `textFormat: Text.PlainText`; the test
`qml_text::every_label_that_shows_a_value_is_plain_text` (`crates/desktop/src/qml_text.rs`) fails on one that doesn't.

## Where the window tells

Every report of the window, and what it is: the notice it posts (the inbox logs it, its target the notice's source),
or a log record when it stays a diagnostic. A state is cleared by the next success of the same operation.

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
| `providers.rs` (4) | `providers`, warning | An image request failed, or wasn't one | log (S3 records a thumbnail's failure) |
| `animation.rs` `restart` | `animation`, warning | An animation couldn't be read | log |
| `video_source.rs` `play`, `video_player.rs` `play_path` | `video`, warning | A video couldn't be opened | log |
| `yandex.rs` (3) | `yandex`, warning | A search's file, the pump, a thread | log (the panel says it) |
| `log_model.rs` `start` | `log`, warning | The log view's reader didn't start | log |
