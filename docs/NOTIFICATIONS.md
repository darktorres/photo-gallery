# Notifications: the notices and the log

How the app tells the user what happened, and what it records for a developer. The design and its stages are in
[NOTIFICATIONS_PLAN.md](NOTIFICATIONS_PLAN.md); this is the reference for the code as it is. Stage S1 is in: the notice
types, the log, dev mode and the log view. The inbox (S2) and the failure ledger (S3) come next. Until S2, the notices
reach the user as the status line and the log records below, as before.

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
3. From S2, its words: the title and the body entries in `client/locales/en/text.ftl`, which the parity test checks
   against the ids.
4. The call that posts it, from the crate that has the outcome.

A new action is a variant of `Action`, the window's handler for it, and the kinds that offer it in `Notice::actions`.

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
closes the log view in dev mode.

The shortcuts are Qt `Shortcut`s (`GlobalShortcuts.qml`), which Qt matches before the focused item's keys, so they work
from the grid, the viewer and a text field. What a shortcut does is the controller's (`pg_client::controller`): while a
dialog or a menu is open, it is that panel's and the shortcut does nothing; the log view's key closes it when it is open,
and opens it otherwise (closing the inbox first, from S2). Each modal panel reports its opening and closing
(`Shell.panelOpened`, `panelClosed`, by its `PanelKind.qml` code), so the controller knows which are open.

The log view (`LogPanel.qml`, over `LogModel`) shows this run's records and the files', oldest first, following the new
ones. A row is one line; a click shows its whole message and detail below. Escape or Close dismisses it, and Tab stays
in it.

## Plain text

A `Label` or `Text` shows a value that looks like markup as rich text (Qt's `AutoText`), so a name, a path or an error
holding `<b>` would be drawn as HTML. Every one whose text isn't fixed words sets `textFormat: Text.PlainText`; the test
`qml_text::every_label_that_shows_a_value_is_plain_text` (`crates/desktop/src/qml_text.rs`) fails on one that doesn't.

## Where the app logs (S1)

Every log call of the window, with its target and level. The *S2* column is what the inbox makes of it then: the notice
it posts (keyed: a state), or *log* when it stays a diagnostic.

| Site | Target, level | What | S2 |
| --- | --- | --- | --- |
| `lib.rs` `run` | `session`, info | The run's start: the build and the data folder | log |
| `lib.rs` `run` | `config`, warning | `PG_THUMB_CAP_GB` didn't parse | log |
| `panic_log.rs` hook | `panic`, error | A panic, with its backtrace | log (the box, for a fatal one) |
| `app.rs` `open` | `app`, warning | The app's notes (preferences, a rebuilt catalog, the thumbnail cache) | the notes' notices |
| `app.rs` `stop` | `app`, warning | The background work didn't stop | log |
| `controller.rs` `Core` (3) | `record`, warning | `PG_RECORD` couldn't be written | log |
| `command_line.rs` `yandex_source` | `yandex`, info | `--yandex-fixture` answers from a recording | log |
| `gallery/state.rs` `with_details` | `gallery`, warning | A file's row couldn't be read | `RowReadFailed` |
| `gallery/state.rs` `fresh_entry` | `gallery`, warning | A file's row couldn't be read again | `ReloadFailed` |
| `gallery/model.rs` `open_listing` | `gallery`, warning | The list couldn't be read | `ListingFailed` |
| `gallery/edits.rs` `rotate`, `set_rating`, `set_border_marked` | `edit`, warning | An edit couldn't be saved | `EditFailed` |
| `filters.rs` `count_facets` | `filters`, warning | The filter bar's counts couldn't be read | `FacetsFailed` |
| `folders.rs` `reload` | `folders`, warning | The folder tree couldn't be read | `TreeFailed` |
| `shell.rs` `view_names`, `save_view`, `delete_view` | `views`, warning | The saved views couldn't be listed, saved, deleted | `ViewsFailed` |
| `shell.rs` `execute` (`SaveSession`) | `session`, warning | The session couldn't be saved | `SessionNotSaved` |
| `picker.rs` `pick` | `picker`, warning | The folder dialog failed (the call, the start folder) | `PickerFailed` |
| `picker.rs` `report_unlisted` | `picker`, warning | The app's folder dialog couldn't list a folder | log (the dialog says it) |
| `library.rs` `load_roots` | `library`, warning | The library's folders couldn't be read | `FoldersUnreadable` |
| `library.rs` `start` | `library`, error | The background work didn't start | `LibraryUnavailable` |
| `library.rs` `start` | `library`, warning | The change pump didn't start | log |
| `providers.rs` (4) | `providers`, warning | An image request failed, or wasn't one | log (S3 records a thumbnail's failure) |
| `animation.rs` `restart` | `animation`, warning | An animation couldn't be read | log |
| `video_source.rs` `play`, `video_player.rs` `play_path` | `video`, warning | A video couldn't be opened | log |
| `yandex.rs` (3) | `yandex`, warning | A search's file, the pump, a thread | log (the panel says it) |
| `log_model.rs` `start` | `log`, warning | The log view's reader didn't start | log |
