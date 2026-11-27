//! The window's inbox (InboxPanel.qml, the bell and the toast): what `pg_app::Inbox` lists, newest first, worded by
//! `pg_wording`. `start` subscribes to the inbox; a thread hands what it publishes to the Qt thread in passes, one
//! queued at a time, as the library's changes are (`library.rs`): the latest listing, and the toasts that came with it.
//!
//! A new listing is applied as the changes from the rows shown (`list_diff`, keyed by each entry's identity,
//! `EntryId::entry`): rows removed, inserted and moved, and the rows whose entry changed in place told so. So the panel
//! keeps its selection and the details it has open when a notice comes in. The window never reads the catalog for the
//! inbox on its thread: Copy reads a ledger's files through the window's reader (`pg_app::Session::copy_details`), and
//! copies when the read is delivered.
//!
//! A mark or a dismissal changes the rows here at once and is queued for the inbox's writer, whose next listing
//! confirms it; it names the entries as they were displayed (a row at its revision, a ledger up to its newest serial),
//! never one the window wasn't given. While the panel is open the inbox reads the ledgers' counts
//! (`Inbox::set_viewing`); while it is open and the window active, what is shown is marked read, a new listing's entries
//! too.
//!
//! ## The window
//!
//! `Notices` (`src/notices.rs`): the status bar's Inbox button shows the unread count and opens the inbox; the newest toast
//! is in the status bar (an info goes after 6 s, a warning stays until it is replaced or the inbox opens). The inbox
//! (`InboxPanel.qml`) lists the entries, newest first. The list is keyed by entry and updated by a diff of each listing
//! (`list_diff.rs`), so a new notice keeps the selection and the open details, and it is driven from the keyboard. Opening the inbox marks read the entries it displays, and entries arriving while it is open and
//! the window active are marked when they are shown. A row's details have Copy (to the clipboard) and Dismiss; Clear all
//! dismisses the entries displayed. **Ctrl+Shift+N** opens and closes it. `--inbox=1` opens it for the screenshots, and
//! `--inbox-expand=<ledger>` opens a ledger's entry, by the ledger's code.
//!
//! **The status bar's link.** The job line says "N failed" as a link only while the inbox lists that ledger (a
//! `passes` property with a change signal, a bit per ledger with a listed entry or an `unreadable-files` state); otherwise it
//! is plain text. A click opens the inbox expanded at that entry, and an entry that appears later expands when it does. A
//! pass that writes or clears ledger rows calls `Control::ledger_changed`, whose hook the runner sets to the inbox's poke
//! (the thumbnails' and the borders' passes after each write or clear that changed rows, the import after every batch,
//! since its rows move by trigger, and the scan); pokes coalesce in `Work`. The thumbnails' pass flushes at least every
//! 2 s, so the entry follows a failing file within about 2 s.
//!
//! **The inbox's own reads leave the GUI thread.** A ledger entry's list (its newest files, or its folders, with the
//! counts read together), what Copy adds to an entry's words, and the folder a Reveal opens are read by the window's
//! `Reader` (the app's private `reader` module; kinds `FailedFiles`, `CopyDetails` and `RevealFolder`), which keeps only the newest
//! request of each kind per session and cancels a stale one; each kind is its own, so none cancels another. Each caller
//! asks, then goes on when its read is delivered: the list shows "loading", Copy copies, Reveal lists the folder or toasts
//! `FileNoLongerInLibrary`. These reads are the user's, so waiting behind a listing that is running is acceptable. The rule: what the notifications added or use reads nothing on the GUI thread. The rest of the window's catalog work on that thread is left for a separate change.
//!
//! **Reveal waits for its own listing.** The controller stamps each `Command::OpenList` with a generation from one
//! counter of its own; the grid carries it (through `GalleryModel::open_listing` and `Grid::arrive`, never through QML),
//! and `Shell::list_shown` reports the generation shown. A reveal is consumed only by its own listing's `ListShown`, and
//! navigating away drops it.
//!
//! ## Where the window tells
//!
//! Every report of the window, and what it is: the notice it posts (the inbox logs it, its target the notice's source),
//! or a log record when it stays a diagnostic. A state is cleared by its resolver (the table in `notice/resolver_table.golden`).
//!
//! | Site | Target, level | What | Recorded as |
//! | --- | --- | --- | --- |
//! | `lib.rs` `run` | `session`, info | The run's start: the build and the data folder | log |
//! | `lib.rs` `run` | `config`, warning | `PG_THUMB_CAP_GIB` didn't parse | log |
//! | `panic_log.rs` hook | `panic`, error | A panic, with its backtrace | log (the box, for a fatal one) |
//! | `app.rs` `stop` | `app`, warning | The background work didn't stop | log |
//! | `controller.rs` `Core` (3) | `record`, warning | `PG_RECORD` couldn't be written | log |
//! | `pg_app` `yandex::Yandex::new` (from `--yandex-fixture`) | `yandex`, info | A recording answers the searches | log |
//! | `pg_app` `App::stop` | `yandex`, warning | A search, a save or a fetch still ran at the stop | log |
//! | `bridges/gallery_state.rs` `read_details` | `gallery`, warning | A file's row couldn't be read | `RowReadFailed` |
//! | `bridges/gallery_state.rs` `fresh_entry` | `gallery`, warning | A file's row couldn't be read again | `ReloadFailed` |
//! | `gallery/model.rs` `open_listing` | `gallery`, warning | The list couldn't be read | `ListingFailed` |
//! | `gallery/edits.rs` `rotate`, `set_rating`, `set_border_marked` | `edit`, warning | An edit couldn't be saved | `EditFailed` |
//! | `filters.rs` `count_facets` | `filters`, warning | The filter bar's counts couldn't be read | `FacetsFailed` |
//! | `folders.rs` `reload` | `folders`, warning | The folder tree couldn't be read | `TreeFailed` |
//! | `shell.rs` `view_names`, `save_view`, `delete_view` | `views`, warning | The saved views couldn't be listed, saved, deleted | `ViewsFailed` |
//! | `shell.rs` `execute` (`SaveSession`) | `session`, warning | The session couldn't be saved | `SessionNotSaved` |
//! | `picker.rs` `pick` | `picker`, warning | The folder dialog failed (the call, the start folder) | `PickerFailed` |
//! | `picker.rs` `report_unlisted` | `picker`, warning | The app's folder dialog couldn't list a folder | `PickerFailed` |
//! | `library.rs` `load_roots` | `library`, warning | The library's folders couldn't be read | `FoldersUnreadable` |
//! | `library.rs` `start` | `library`, error | The background work didn't start | `LibraryUnavailable` |
//! | `library.rs` the change pump's start | `library`, error | The window can't see the library's changes | `ChangesUnseen` |
//! | `library.rs` `start` | `library`, warning | The change pump didn't start | log |
//! | `providers.rs` (3) | `providers`, warning | An image request failed, or wasn't one | log (a thumbnail that doesn't decode is also recorded in the ledger, by `pg_app::Images`) |
//! | `animation.rs` `restart` | `animation`, warning | An animation couldn't be read | log |
//! | `video_source.rs` `play`, `video_player.rs` `play_path` | `video`, warning | A video couldn't be opened | log |
//! | `yandex.rs` (3) | `yandex`, warning | A search's file, the pump, a thread | log (the panel says it) |
//! | `log_model.rs` `start` | `log`, warning | The log view's reader didn't start | log |
//!
//! The writer's failures (`write-failed`, `write-stopped`) and a dropped edit (`write-dropped`, an event: a lost edit is a
//! fact, not a condition) are posted by `pg_writeback` and `pg_app::Library`; the window's `refresh_writes` only shows the writer's status.

use crate::bridges::notices_bridge::qobject;
use crate::rows::qt_int;

use core::pin::Pin;
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use crate::pump::Pending;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QByteArray, QHash, QHashPair_i32_QByteArray, QList, QModelIndex, QString, QVariant};
use pg_app::{FailedItems, InboxEvent, InboxListing};
use pg_model::inbox::{InboxEntry, InboxEntryId};
use pg_model::notice::{Action, LedgerKind, Level, Notice};
use pg_model::rules::PathRules;
use pg_wording::Message;

use crate::app;
use crate::ledgers::{kind_code, kind_of};
use crate::list_diff::{self, Op};

thread_local! {
    /// Each ledger's acknowledgement mark as the user last saw it (the Qt thread's): while the inbox is closed, the
    /// latest listing's; while it is open, the one it opened with (a ledger that appears meanwhile keeps its first).
    /// Its list and its Copy say what was new then, whatever marking it read does meanwhile.
    static SEEN_AT_OPEN: RefCell<BTreeMap<LedgerKind, i64>> = const { RefCell::new(BTreeMap::new()) };
    /// The number of the latest copy asked for (kept notice or ledger). A ledger's copy writes the clipboard only if no
    /// copy was asked for after it: its read can finish after a later copy. Read and written on the Qt thread.
    static COPY_SEQ: Cell<u64> = const { Cell::new(0) };
}

/// `kind`'s mark as the user last saw it, if it was listed (`None`: the acknowledgement as it is now).
pub(crate) fn seen_at_open(kind: LedgerKind) -> Option<i64> {
    SEEN_AT_OPEN.with_borrow(|kept| kept.get(&kind).copied())
}

/// Keep the ledgers' marks of `rows`: all of them (`fresh`, the inbox closed), or only the ledgers not kept yet.
fn keep_seen(rows: &[InboxEntry], fresh: bool) {
    SEEN_AT_OPEN.with_borrow_mut(|kept| {
        if fresh {
            kept.clear();
        }
        for (kind, seen_to) in rows.iter().filter_map(|e| Some((ledger_kind(e.id)?, e.seen_to?))) {
            kept.entry(kind).or_insert(seen_to);
        }
    });
}

const ROLE_TITLE: i32 = 256;
const ROLE_BODY: i32 = 257;
const ROLE_SOURCE: i32 = 258;
const ROLE_LEVEL: i32 = 259;
const ROLE_UPDATED: i32 = 260;
const ROLE_COUNT: i32 = 261;
const ROLE_READ: i32 = 262;
const ROLE_PASS: i32 = 263;
const ROLE_CAN_RETRY: i32 = 264;
const ROLE_CAN_SHOW_FILES: i32 = 265;
const ROLE_KEY: i32 = 266;
const ROLE_SINCE_DISMISSED: i32 = 267;
const ROLE_SERIAL: i32 = 268;

/// The most failed files (or folders) Copy puts on the clipboard for a ledger's entry.
const COPIED_FILES: usize = 1000;

/// The ledger the entry is, if it is one.
fn ledger_kind(id: InboxEntryId) -> Option<LedgerKind> {
    match id {
        InboxEntryId::Ledger { kind, .. } => Some(kind),
        InboxEntryId::Row { .. } => None,
    }
}

/// The entry's key in QML (the `key` role, `rowOfKey`): its identity whatever revision it was listed at
/// (`EntryId::entry`), as text: `row:<id>:<uid>` for a kept notice, `ledger:<code>` for a ledger.
fn key_of(id: InboxEntryId) -> String {
    match id.entry() {
        InboxEntryId::Row { id, uid, .. } => format!("row:{id}:{uid}"),
        InboxEntryId::Ledger { kind, .. } => ledger_key(kind),
    }
}

/// The key of `kind`'s entry ([`key_of`]).
fn ledger_key(kind: LedgerKind) -> String {
    format!("ledger:{}", kind.key())
}

/// The ledgers that something listed is about, as `Notices.passes`' bits (bit [`kind_code`]): a ledger's own entry, or a
/// pass's unreadable files (`unreadable-files`), which the status bar's "N failed" counts too.
fn passes_of(entries: &[InboxEntry]) -> i32 {
    entries
        .iter()
        .filter_map(|entry| match (&entry.notice, entry.id) {
            (_, InboxEntryId::Ledger { kind, .. }) => Some(kind),
            (Notice::UnreadableFiles { pass, .. }, InboxEntryId::Row { .. }) => Some(LedgerKind::File(*pass)),
            _ => None,
        })
        .filter_map(|kind| u32::try_from(kind_code(Some(kind))).ok())
        .fold(0, |bits, bit| bits | 1_i32.checked_shl(bit).unwrap_or(0))
}

/// What Copy puts on the clipboard: the notice on one line, then for a ledger what its read gave, one a line: its files,
/// or its folders under the library folder that holds them.
fn copy_words(notice: &Notice, rules: PathRules, items: Option<&FailedItems>) -> String {
    let mut words = pg_wording::notice_line(notice, rules);
    match items {
        None => {}
        Some(FailedItems::Files(files)) => {
            for file in files {
                words.push('\n');
                let path = rules.display(&file.path);
                words.push_str(&pg_wording::render(&Message::FailedFileLine { path, error: file.error.clone() }));
            }
        }
        Some(FailedItems::Folders(folders)) => {
            let mut root = None;
            for folder in folders {
                if root != Some(folder.root) {
                    root = Some(folder.root);
                    words.push('\n');
                    let root = rules.display(&folder.root_path);
                    words.push_str(&pg_wording::render(&Message::FailedFolderGroup { root }));
                }
                words.push_str("\n  ");
                words.push_str(&pg_wording::render(&Message::FailedFolderLine {
                    path: rules.display(&folder.path),
                    why: pg_wording::render(&Message::FailedFolderReason { reason: folder.failure.key().to_string() }),
                    error: folder.error.clone(),
                }));
            }
        }
    }
    words
}

/// What the inbox published that the Qt thread hasn't shown (gathered into `pump::Pending`).
#[derive(Default)]
struct InboxChanges {
    listing: Option<Arc<InboxListing>>,
    toasts: Vec<Notice>,
}

impl qobject::Notices {
    pub(crate) fn data(&self, index: &QModelIndex, role: i32) -> QVariant {
        let Some(entry) = usize::try_from(index.row()).ok().and_then(|row| self.rows.get(row)) else {
            return QVariant::default();
        };
        let rules = pg_platform::native::PATH_RULES;
        let notice = &entry.notice;
        match role {
            ROLE_TITLE => QVariant::from(&QString::from(&pg_wording::render(&Message::NoticeTitle {
                notice: notice.clone(),
                rules,
            }))),
            ROLE_BODY => QVariant::from(&QString::from(&pg_wording::render(&Message::NoticeBody {
                notice: notice.clone(),
                rules,
            }))),
            ROLE_SOURCE => {
                QVariant::from(&QString::from(&pg_wording::render(&Message::SourceLabel { source: notice.source() })))
            }
            ROLE_LEVEL => QVariant::from(&notice.level().code()),
            // Unix seconds, as a number QML makes a `Date` of.
            ROLE_UPDATED => QVariant::from(&pg_model::num::signed(entry.updated)),
            // How many times a kept notice was posted, how many files a ledger holds; -1 when not counted (a ledger's
            // counts are read only while the inbox is viewed), which shows nothing.
            ROLE_COUNT => QVariant::from(&entry.counts.map_or(-1, |c| qt_int(c.total))),
            ROLE_READ => QVariant::from(&entry.read),
            // A ledger's entry lists its files (or folders) when it is opened (`FailedFiles.show`).
            ROLE_PASS => QVariant::from(&kind_code(ledger_kind(entry.id))),
            ROLE_CAN_RETRY => QVariant::from(&notice.actions().contains(&Action::Retry)),
            ROLE_CAN_SHOW_FILES => QVariant::from(&notice.actions().contains(&Action::ShowFailedFiles)),
            ROLE_KEY => QVariant::from(&QString::from(&key_of(entry.id))),
            // A ledger back after a dismissal: how many came since ("3 since you dismissed it"); empty otherwise.
            ROLE_SINCE_DISMISSED => {
                let since = match (entry.id, entry.counts) {
                    // Nothing says none: a ledger entry with no failure since its dismissal has no line.
                    (InboxEntryId::Ledger { .. }, Some(counts)) if counts.since_dismissed > 0 => {
                        let count = usize::try_from(counts.since_dismissed).unwrap_or(usize::MAX);
                        pg_wording::render(&Message::LedgerSinceDismissed { count })
                    }
                    _ => String::new(),
                };
                QVariant::from(&QString::from(&since))
            }
            // A ledger's newest failure as listed (0 for a kept notice): the expanded list is read again when it moves.
            ROLE_SERIAL => QVariant::from(&match entry.id {
                InboxEntryId::Ledger { seen, .. } => pg_model::num::signed(seen),
                InboxEntryId::Row { .. } => 0.0,
            }),
            _ => QVariant::default(),
        }
    }

    pub(crate) fn role_names(&self) -> QHash<QHashPair_i32_QByteArray> {
        let mut roles = QHash::<QHashPair_i32_QByteArray>::default();
        for (role, name) in [
            (ROLE_TITLE, "title"),
            (ROLE_BODY, "body"),
            (ROLE_SOURCE, "source"),
            (ROLE_LEVEL, "level"),
            (ROLE_UPDATED, "updated"),
            (ROLE_COUNT, "count"),
            (ROLE_READ, "read"),
            (ROLE_PASS, "pass"),
            (ROLE_CAN_RETRY, "canRetry"),
            (ROLE_CAN_SHOW_FILES, "canShowFiles"),
            (ROLE_KEY, "key"),
            (ROLE_SINCE_DISMISSED, "sinceDismissed"),
            (ROLE_SERIAL, "serial"),
        ] {
            roles.insert(role, QByteArray::from(name));
        }
        roles
    }

    pub(crate) fn row_count(&self, _parent: &QModelIndex) -> i32 {
        qt_int(self.rows.len())
    }

    pub(crate) fn start(mut self: Pin<&mut Self>) {
        if self.started {
            return;
        }
        self.as_mut().rust_mut().started = true;
        let inbox = app::get().inbox();
        // Subscribed before the listing is read, so a change meanwhile comes as an event.
        let events = inbox.subscribe();
        self.as_mut().show(&inbox.listing());
        // Without its thread the window isn't told the inbox's changes: the log says why.
        let follower = crate::pump::start(
            "inbox view",
            self.qt_thread(),
            move || events.recv(),
            |changes: &mut InboxChanges, event: InboxEvent| match event {
                InboxEvent::Changed(listing) => changes.listing = Some(listing),
                InboxEvent::Toast(notice) => changes.toasts.push(notice),
            },
            |notices, pending| notices.on_pass(&pending),
        );
        if let Err(e) = follower {
            pg_log::warn("inbox", &format!("the inbox view's follower didn't start: {e:#}"));
        }
    }

    /// One pass on the Qt thread: the latest listing, and the newest toast.
    fn on_pass(mut self: Pin<&mut Self>, pending: &Mutex<Pending<InboxChanges>>) {
        let InboxChanges { listing, toasts } = Pending::take(pending);
        if let Some(listing) = listing {
            self.as_mut().show(&listing);
        }
        if let Some(notice) = toasts.last() {
            let words = pg_wording::render(&Message::NoticeTitle {
                notice: notice.clone(),
                rules: pg_platform::native::PATH_RULES,
            });
            self.as_mut().set_toast_shown(QString::from(&words), notice.level().code());
        }
    }

    /// Show `listing`: the changes from the rows shown (`list_diff`), then each row whose entry changed in place (a new
    /// revision, its counts, read) told so. Shown while the panel is open in an active window, what is new is read.
    fn show(mut self: Pin<&mut Self>, listing: &InboxListing) {
        let old: Vec<InboxEntryId> = self.rows.iter().map(|e| e.id.entry()).collect();
        let new: Vec<InboxEntryId> = listing.entries.iter().map(|e| e.id.entry()).collect();
        let root = QModelIndex::default();
        for op in list_diff::diff(&old, &new) {
            match op {
                Op::Remove { at, count } => {
                    self.as_mut().begin_remove_rows(&root, qt_int(at), qt_int(at + count - 1));
                    list_diff::apply(&mut self.as_mut().rust_mut().rows, op, &listing.entries);
                    self.as_mut().end_remove_rows();
                }
                Op::Insert { at, count } => {
                    self.as_mut().begin_insert_rows(&root, qt_int(at), qt_int(at + count - 1));
                    list_diff::apply(&mut self.as_mut().rust_mut().rows, op, &listing.entries);
                    self.as_mut().end_insert_rows();
                }
                Op::Move { from, to } => {
                    // A row moves up only (`to < from`), which Qt always takes.
                    let moved = self.as_mut().begin_move_rows(&root, qt_int(from), qt_int(from), &root, qt_int(to));
                    debug_assert!(moved, "Qt takes a row that moves up");
                    list_diff::apply(&mut self.as_mut().rust_mut().rows, op, &listing.entries);
                    self.as_mut().end_move_rows();
                }
            }
        }
        for (at, entry) in listing.entries.iter().enumerate() {
            if self.rows.get(at) != Some(entry) {
                if let Some(row) = self.as_mut().rust_mut().rows.get_mut(at) {
                    *row = entry.clone();
                }
                self.as_mut().row_changed(at);
            }
        }
        self.as_mut().set_unread_count(qt_int(listing.unread));
        let passes = passes_of(&self.rows);
        self.as_mut().set_passes_shown(passes);
        keep_seen(&self.rows, !self.viewing);
        if self.marking {
            self.as_mut().mark_shown_read();
        }
        self.as_mut().refresh_selection();
    }

    /// The row at `at` changed in place: every role is read again.
    fn row_changed(mut self: Pin<&mut Self>, at: usize) {
        let index = self.index(qt_int(at), 0, &QModelIndex::default());
        self.as_mut().data_changed(&index, &index, &QList::default());
    }

    fn set_unread_count(mut self: Pin<&mut Self>, unread: i32) {
        if self.unread != unread {
            self.as_mut().rust_mut().unread = unread;
            self.unread_changed();
        }
    }

    fn set_passes_shown(mut self: Pin<&mut Self>, passes: i32) {
        if self.passes != passes {
            self.as_mut().rust_mut().passes = passes;
            self.passes_changed();
        }
    }

    fn set_toast_shown(mut self: Pin<&mut Self>, words: QString, level: i32) {
        self.as_mut().rust_mut().toast = words;
        self.as_mut().rust_mut().toast_level = level;
        self.as_mut().toast_level_changed();
        self.toast_changed();
    }

    /// Mark the unread entries shown read, as they were displayed: none the window wasn't given. Nothing is sent when
    /// every entry shown is read.
    fn mark_shown_read(mut self: Pin<&mut Self>) {
        let unread: Vec<usize> = self.rows.iter().enumerate().filter(|(_, e)| !e.read).map(|(at, _)| at).collect();
        if unread.is_empty() {
            return;
        }
        let ids: Vec<InboxEntryId> = unread.iter().filter_map(|&at| self.rows.get(at)).map(|e| e.id).collect();
        for &at in &unread {
            if let Some(entry) = self.as_mut().rust_mut().rows.get_mut(at) {
                entry.read = true;
            }
            self.as_mut().row_changed(at);
        }
        let left = self.unread.saturating_sub(qt_int(unread.len()));
        self.as_mut().set_unread_count(left);
        app::get().inbox().mark_read(ids);
    }

    /// The panel is `open` or not, in a window that is `active` or not: the inbox reads the ledgers' counts while it is
    /// open, and what is shown is read while it is open in an active window.
    pub(crate) fn set_viewing(mut self: Pin<&mut Self>, open: bool, active: bool) {
        if self.viewing != open {
            self.as_mut().rust_mut().viewing = open;
            // The marks the inbox opens with stay while it is open (opening marks it read: what was new stays so).
            if !open {
                keep_seen(&self.rows, true);
                // A warning raised while the inbox was open is in its list, and read by it: the toast it left behind
                // is dismissed too, rather than outliving the panel.
                if Level::from_code(self.toast_level) >= Level::Warning {
                    self.as_mut().dismiss_toast();
                }
            }
            app::get().inbox().set_viewing(open);
        }
        self.as_mut().rust_mut().marking = open && active;
        if self.marking {
            self.mark_shown_read();
        }
    }

    pub(crate) fn dismiss(mut self: Pin<&mut Self>, row: i32) {
        let Some(at) = usize::try_from(row).ok().filter(|&at| at < self.rows.len()) else { return };
        let root = QModelIndex::default();
        self.as_mut().begin_remove_rows(&root, row, row);
        let entry = self.as_mut().rust_mut().rows.remove(at);
        self.as_mut().end_remove_rows();
        if !entry.read {
            let unread = self.unread.saturating_sub(1);
            self.as_mut().set_unread_count(unread);
        }
        let passes = passes_of(&self.rows);
        self.as_mut().set_passes_shown(passes);
        self.as_mut().refresh_selection();
        app::get().inbox().dismiss(entry.id);
    }

    /// "Clear all": the entries shown, as they were displayed; none the window wasn't given.
    pub(crate) fn dismiss_all(mut self: Pin<&mut Self>) {
        let ids: Vec<InboxEntryId> = self.rows.iter().map(|e| e.id).collect();
        let unread = self.unread.saturating_sub(qt_int(self.rows.iter().filter(|e| !e.read).count()));
        self.as_mut().begin_reset_model();
        self.as_mut().rust_mut().rows.clear();
        self.as_mut().end_reset_model();
        self.as_mut().set_unread_count(unread);
        self.as_mut().set_passes_shown(0);
        self.as_mut().refresh_selection();
        app::get().inbox().dismiss_all(ids);
    }

    /// Select the entry at `row` (none for a row past the list). Its details are what the panel shows.
    pub(crate) fn select(mut self: Pin<&mut Self>, row: i32) {
        let key = usize::try_from(row)
            .ok()
            .and_then(|at| self.rows.get(at))
            .map_or_else(QString::default, |entry| QString::from(&key_of(entry.id)));
        self.as_mut().rust_mut().selected_key = key;
        self.as_mut().selected_key_changed();
        self.as_mut().refresh_selection();
    }

    /// Clear the selection, and close its details.
    pub(crate) fn unselect(mut self: Pin<&mut Self>) {
        self.as_mut().rust_mut().selected_key = QString::default();
        self.as_mut().rust_mut().details_open = false;
        self.as_mut().selected_key_changed();
        self.as_mut().details_open_changed();
        self.as_mut().refresh_selection();
    }

    /// Open or close the selected entry's details.
    pub(crate) fn set_details_open(mut self: Pin<&mut Self>, open: bool) {
        if self.details_open != open {
            self.as_mut().rust_mut().details_open = open;
            self.as_mut().details_open_changed();
            self.as_mut().refresh_selection();
        }
    }

    /// Dismiss the selected entry, which clears the selection.
    pub(crate) fn dismiss_selected(mut self: Pin<&mut Self>) {
        let row = self.selected_row;
        if row < 0 {
            return;
        }
        self.as_mut().unselect();
        self.as_mut().dismiss(row);
    }

    /// Try the selected entry's failed files again, when it can be retried.
    pub(crate) fn retry_selected(&self) {
        if self.selected_row >= 0 && self.selected_can_retry {
            self.retry(self.selected_row);
        }
    }

    /// The selection as the panel reads it. It is held by key, so it follows its entry as the rows move, and it is
    /// dropped, with its details, when the entry is gone. Then the failed files of a ledger are asked for, while its
    /// details are open, once per entry and newest failure.
    fn refresh_selection(mut self: Pin<&mut Self>) {
        let key = self.selected_key.to_string();
        let row = if key.is_empty() { None } else { self.rows.iter().position(|e| key_of(e.id) == key) };
        if row.is_none() && !key.is_empty() {
            self.as_mut().rust_mut().selected_key = QString::default();
            self.as_mut().rust_mut().details_open = false;
            self.as_mut().selected_key_changed();
            self.as_mut().details_open_changed();
        }
        let shown = match row.and_then(|at| self.rows.get(at).map(|entry| (at, entry))) {
            Some((at, entry)) => {
                let rules = pg_platform::native::PATH_RULES;
                let notice = &entry.notice;
                let serial = match entry.id {
                    InboxEntryId::Ledger { seen, .. } => pg_model::num::signed(seen),
                    InboxEntryId::Row { .. } => 0.0,
                };
                (
                    qt_int(at),
                    QString::from(&pg_wording::render(&Message::NoticeBody { notice: notice.clone(), rules })),
                    kind_code(ledger_kind(entry.id)),
                    serial,
                    notice.actions().contains(&Action::Retry),
                    notice.actions().contains(&Action::ShowFailedFiles),
                )
            }
            None => (-1, QString::default(), -1, 0.0, false, false),
        };
        let (row_index, text, pass, serial, retry, files) = shown;
        self.as_mut().rust_mut().selected_row = row_index;
        self.as_mut().selected_row_changed();
        self.as_mut().rust_mut().selected_text = text;
        self.as_mut().selected_text_changed();
        self.as_mut().rust_mut().selected_pass = pass;
        self.as_mut().selected_pass_changed();
        self.as_mut().rust_mut().selected_serial = serial;
        self.as_mut().selected_serial_changed();
        self.as_mut().rust_mut().selected_can_retry = retry;
        self.as_mut().selected_can_retry_changed();
        self.as_mut().rust_mut().selected_can_show_files = files;
        self.as_mut().selected_can_show_files_changed();
        self.as_mut().maybe_request_files();
    }

    /// The failed files of the selected ledger are to be listed while its details are open: once per entry and newest
    /// failure (`filesRequested`), so the list is read again only when what it is of changes.
    fn maybe_request_files(mut self: Pin<&mut Self>) {
        let pass = if self.details_open && self.selected_row >= 0 { self.selected_pass } else { -1 };
        let key = if pass >= 0 { self.selected_key.to_string() } else { String::new() };
        let serial = self.selected_serial;
        let (requested_key, requested_serial) = self.requested.clone();
        if key == requested_key.to_string() && (key.is_empty() || serial == requested_serial) {
            return;
        }
        self.as_mut().rust_mut().requested = (QString::from(&key), serial);
        self.as_mut().files_requested(pass);
    }

    /// Put the entry at `row` on the clipboard: at once for a kept notice; for a ledger, once its files (or folders) are
    /// read off the window's thread.
    pub(crate) fn copy(&self, row: i32) {
        let Some(entry) = usize::try_from(row).ok().and_then(|row| self.rows.get(row)) else { return };
        let rules = pg_platform::native::PATH_RULES;
        let notice = entry.notice.clone();
        // This copy is the latest asked for: a ledger copy still reading does not write the clipboard after it.
        let seq = COPY_SEQ.with(|seq| {
            seq.set(seq.get() + 1);
            seq.get()
        });
        let Some(kind) = ledger_kind(entry.id) else {
            crate::bridges::clipboard::ffi::copy_text(&QString::from(&copy_words(&notice, rules, None)));
            return;
        };
        let notices = self.qt_thread();
        app::session().copy_details(kind, COPIED_FILES, seen_at_open(kind), move |read| {
            let items = match read {
                Ok(failed) => Ok(failed.items),
                Err(e) => {
                    pg_log::warn("inbox", &format!("the failed files to copy couldn't be read: {e:#}"));
                    Err(app::error_text(&e))
                }
            };
            // On the Qt thread, which the clipboard and the inbox's notices are; fails only when the window is gone.
            let _ = notices.queue(move |_| {
                let items = match items {
                    Ok(items) => Some(items),
                    Err(error) => {
                        // The copy holds the headline alone: the user is told why it is not the whole entry.
                        app::post(Notice::ListingFailed { error });
                        None
                    }
                };
                if COPY_SEQ.with(Cell::get) == seq {
                    let words = copy_words(&notice, rules, items.as_ref());
                    crate::bridges::clipboard::ffi::copy_text(&QString::from(&words));
                }
            });
        });
    }

    /// The row of the entry with `key` (the `key` role), -1 when the inbox lists none.
    pub(crate) fn row_of_key(&self, key: &QString) -> i32 {
        let key = key.to_string();
        self.rows.iter().position(|e| key_of(e.id) == key).map_or(-1, qt_int)
    }

    /// The key of a ledger's entry (a `pass` code), "" for a code that names none.
    pub(crate) fn pass_key(&self, pass: i32) -> QString {
        kind_of(pass).map_or_else(QString::default, |kind| QString::from(&ledger_key(kind)))
    }

    /// The `pass` code of a ledger's code (`thumbs`, `borders`, `import`, `scan-folders`: `--inbox-expand`), -1 for none.
    pub(crate) fn pass_of_code(&self, code: &QString) -> i32 {
        kind_code(LedgerKind::from_key(&code.to_string()))
    }

    /// Try the failed files of the entry at `row` again (a pass's ledger with Retry): one command of the inbox's writer,
    /// which then wakes the job to make them. The entry stays until the listing after it says what still fails.
    pub(crate) fn retry(&self, row: i32) {
        let Some(entry) = usize::try_from(row).ok().and_then(|row| self.rows.get(row)) else { return };
        let (Some(LedgerKind::File(pass)), true) =
            (ledger_kind(entry.id), entry.notice.actions().contains(&Action::Retry))
        else {
            return;
        };
        app::get().inbox().retry(pass, Box::new(|| app::get().library().wake_job()));
    }

    pub(crate) fn dismiss_toast(self: Pin<&mut Self>) {
        if !self.toast.is_empty() {
            self.set_toast_shown(QString::default(), 0);
        }
    }

    pub(crate) fn poke(&self) {
        app::get().inbox().poke();
    }
}

#[cfg(test)]
mod tests {
    use pg_model::ids::{FileId, RootId};
    use pg_model::ledger::{FailedFile, FailedFolder, FolderFailure};
    use pg_model::notice::{FilePass, LedgerCounts};

    use super::*;

    fn entry(id: InboxEntryId, notice: Notice) -> InboxEntry {
        InboxEntry { id, notice, counts: None, created: 0, updated: 0, read: false, decode_error: None, seen_to: None }
    }

    /// What a ledger's list calls new: past the mark of the latest listing while the inbox is closed, and past the mark
    /// it opened with while it is open (opening marks it read, which moves the listed mark); a ledger that appears
    /// while it is open keeps its first.
    #[test]
    fn a_ledgers_new_rows_are_those_new_when_the_inbox_opened() {
        let ledger = |kind, seen_to| InboxEntry {
            seen_to: Some(seen_to),
            ..entry(InboxEntryId::Ledger { kind, seen: 0 }, Notice::ThumbsFailed { counts: None })
        };
        let thumbs = LedgerKind::File(FilePass::Thumbs);
        keep_seen(&[ledger(thumbs, 3)], true);
        keep_seen(&[ledger(thumbs, 5)], true);
        assert_eq!(seen_at_open(thumbs), Some(5), "closed: the latest listing's");
        // Open: the listing after the mark doesn't move it; a ledger new meanwhile is kept from its first listing.
        keep_seen(&[ledger(thumbs, 9), ledger(LedgerKind::ScanFolders, 2)], false);
        keep_seen(&[ledger(LedgerKind::ScanFolders, 7)], false);
        assert_eq!((seen_at_open(thumbs), seen_at_open(LedgerKind::ScanFolders)), (Some(5), Some(2)));
        // Closed again: the listing's.
        keep_seen(&[ledger(thumbs, 9)], true);
        assert_eq!((seen_at_open(thumbs), seen_at_open(LedgerKind::ScanFolders)), (Some(9), None));
    }

    /// An entry's key is its identity: a new revision of a row, a ledger listed up to a newer serial, keep it.
    #[test]
    fn a_key_names_the_entry_whatever_its_revision() {
        let row = |rev| InboxEntryId::Row { id: 4, uid: 9, rev };
        assert_eq!(key_of(row(1)), key_of(row(2)));
        assert_eq!(key_of(row(1)), "row:4:9");
        assert_ne!(key_of(row(1)), key_of(InboxEntryId::Row { id: 4, uid: 10, rev: 1 }), "a reused id is another row");
        let ledger = |seen| InboxEntryId::Ledger { kind: LedgerKind::ScanFolders, seen };
        assert_eq!(key_of(ledger(3)), key_of(ledger(30)));
        assert_eq!(key_of(ledger(3)), ledger_key(LedgerKind::ScanFolders));
        assert_eq!(key_of(ledger(3)), "ledger:scan-folders");
    }

    /// The status bar links a pass's "N failed" only to what the inbox lists: its ledger's entry, or its unreadable
    /// files.
    #[test]
    fn the_passes_are_the_ledgers_listed_and_the_unreadable_files() {
        let thumbs = LedgerKind::File(FilePass::Thumbs);
        let import = LedgerKind::File(FilePass::Import);
        let row = |id| InboxEntryId::Row { id, uid: id, rev: 0 };
        let listed = [
            entry(InboxEntryId::Ledger { kind: thumbs, seen: 5 }, thumbs.notice(None)),
            entry(row(1), Notice::UnreadableFiles { pass: FilePass::Import, count: 2, drives: Vec::new() }),
            entry(row(2), Notice::RoundFailed { error: "x".into() }),
        ];
        assert_eq!(passes_of(&listed), (1 << kind_code(Some(thumbs))) | (1 << kind_code(Some(import))));
        assert_eq!(passes_of(&listed[2..]), 0);
        let folders = [entry(
            InboxEntryId::Ledger { kind: LedgerKind::ScanFolders, seen: 1 },
            Notice::ScanFoldersFailed { counts: Some(LedgerCounts { new: 1, earlier: 0 }) },
        )];
        assert_eq!(passes_of(&folders), 1 << 3);
    }

    /// Copy puts a ledger's files one a line, and its folders under the library folder that holds them.
    #[test]
    fn copy_lists_a_ledgers_files_and_its_folders_by_library_folder() {
        let rules = PathRules::Windows;
        let p = |text: &str| rules.parse(text).unwrap();
        let notice = LedgerKind::File(FilePass::Thumbs).notice(None);
        let file = FailedFile {
            file_id: FileId::new(1),
            path: p(r"D:\p\a.jpg"),
            error: "not a JPEG".into(),
            retry: false,
            first_seen: 0,
            new: true,
        };
        let words = copy_words(&notice, rules, Some(&FailedItems::Files(vec![file])));
        let mut lines = words.lines();
        assert_eq!(lines.next(), Some(pg_wording::notice_line(&notice, rules).as_str()));
        let shown = format!("{}: not a JPEG", rules.display(&p(r"D:\p\a.jpg")));
        assert_eq!(lines.next(), Some(shown.as_str()));
        assert_eq!(lines.next(), None);

        let folder = |root: i64, root_path: &str, path: &str| FailedFolder {
            root: RootId::new(root),
            root_path: p(root_path),
            path: p(path),
            failure: FolderFailure::Unreadable,
            error: "denied".into(),
            first_seen: 0,
            new: false,
        };
        let folders = vec![folder(1, r"D:\p", r"D:\p\x"), folder(1, r"D:\p", r"D:\p\y"), folder(2, r"E:\q", r"E:\q\z")];
        let notice = LedgerKind::ScanFolders.notice(None);
        let words = copy_words(&notice, rules, Some(&FailedItems::Folders(folders)));
        let lines: Vec<&str> = words.lines().skip(1).collect();
        assert_eq!(
            lines,
            [
                r"In D:\p:",
                r"  D:\p\x: it couldn't be read (denied)",
                r"  D:\p\y: it couldn't be read (denied)",
                r"In E:\q:",
                r"  E:\q\z: it couldn't be read (denied)",
            ]
        );
        // A kept notice, or a ledger whose read failed: the notice alone.
        assert_eq!(copy_words(&notice, rules, None), pg_wording::notice_line(&notice, rules));
    }
}
