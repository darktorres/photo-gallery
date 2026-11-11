//! The window's inbox (InboxPanel.qml, the bell and the toast): what `pg_app::Inbox` lists, newest first, worded by
//! `pg_client::text`. `start` subscribes to the inbox; a thread hands what it publishes to the Qt thread in passes, one
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

pub(crate) mod state;

use crate::notices_bridge::qobject;

use core::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QByteArray, QHash, QHashPair_i32_QByteArray, QList, QModelIndex, QString, QVariant};
use pg_app::reexport::{InboxEntry, InboxEntryId};
use pg_app::{FailedItems, InboxEvent, Listing};
use pg_client::text::{self, Message, render};
use pg_model::notice::{Action, LedgerKind, Level, Notice};
use pg_model::rules::PathRules;

use crate::app;
use crate::failed_files::{kind_code, kind_of};
use crate::list_diff::{self, Op};

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
    format!("ledger:{}", kind.code())
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
    let mut words = text::notice_line(notice, rules);
    match items {
        None => {}
        Some(FailedItems::Files(files)) => {
            for file in files {
                words.push('\n');
                words.push_str(&text::failed_file_line(&rules.display(&file.path), &file.error));
            }
        }
        Some(FailedItems::Folders(folders)) => {
            let mut root = None;
            for folder in folders {
                if root != Some(folder.root) {
                    root = Some(folder.root);
                    words.push('\n');
                    words.push_str(&text::failed_folder_group(&rules.display(&folder.root_path)));
                }
                words.push_str("\n  ");
                words.push_str(&text::failed_folder_line(
                    &rules.display(&folder.path),
                    folder.failure.code(),
                    &folder.error,
                ));
            }
        }
    }
    words
}

/// What the inbox published that the Qt thread hasn't shown, and whether a pass is queued for it.
#[derive(Default)]
struct Pending {
    listing: Option<Arc<Listing>>,
    toasts: Vec<Notice>,
    queued: bool,
}

fn lock(pending: &Mutex<Pending>) -> MutexGuard<'_, Pending> {
    pending.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The `level` role and `toastLevel`: 0 info, 1 warning, 2 error.
fn level_code(level: Level) -> i32 {
    match level {
        Level::Info => 0,
        Level::Warning => 1,
        Level::Error => 2,
    }
}

/// A count as QML takes it.
fn qt_int(n: impl TryInto<i32>) -> i32 {
    n.try_into().unwrap_or(i32::MAX)
}

impl qobject::Notices {
    pub(crate) fn data(&self, index: &QModelIndex, role: i32) -> QVariant {
        let Some(entry) = usize::try_from(index.row()).ok().and_then(|row| self.rows.get(row)) else {
            return QVariant::default();
        };
        let rules = pg_platform::native::PATH_RULES;
        let notice = &entry.notice;
        match role {
            ROLE_TITLE => {
                QVariant::from(&QString::from(&render(&Message::NoticeTitle { notice: notice.clone(), rules })))
            }
            ROLE_BODY => {
                QVariant::from(&QString::from(&render(&Message::NoticeBody { notice: notice.clone(), rules })))
            }
            ROLE_SOURCE => QVariant::from(&QString::from(&render(&Message::SourceLabel { source: notice.source() }))),
            ROLE_LEVEL => QVariant::from(&level_code(notice.level())),
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
                    (InboxEntryId::Ledger { .. }, Some(counts)) => text::since_dismissed(counts.since_dismissed),
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
        let thread = self.qt_thread();
        let pending: Arc<Mutex<Pending>> = Arc::default();
        let follower = std::thread::Builder::new().name("inbox view".into()).spawn(move || {
            while let Some(event) = events.recv() {
                let queue = {
                    let mut pending = lock(&pending);
                    match event {
                        InboxEvent::Changed(listing) => pending.listing = Some(listing),
                        InboxEvent::Toast(notice) => pending.toasts.push(notice),
                    }
                    !std::mem::replace(&mut pending.queued, true)
                };
                if queue {
                    let pass = Arc::clone(&pending);
                    // Fails only when the window is gone: the follower ends with it.
                    if thread.queue(move |notices| notices.on_pass(pass)).is_err() {
                        break;
                    }
                }
            }
        });
        if let Err(e) = follower {
            pg_log::warn("inbox", &format!("the inbox view's follower didn't start: {e}"));
        }
    }

    /// One pass on the Qt thread: the latest listing, and the newest toast.
    fn on_pass(mut self: Pin<&mut Self>, pending: Arc<Mutex<Pending>>) {
        let (listing, toasts) = {
            let mut pending = lock(&pending);
            pending.queued = false;
            (pending.listing.take(), std::mem::take(&mut pending.toasts))
        };
        if let Some(listing) = listing {
            self.as_mut().show(&listing);
        }
        if let Some(notice) = toasts.last() {
            let words =
                render(&Message::NoticeTitle { notice: notice.clone(), rules: pg_platform::native::PATH_RULES });
            self.as_mut().set_toast_shown(QString::from(&words), level_code(notice.level()));
        }
    }

    /// Show `listing`: the changes from the rows shown (`list_diff`), then each row whose entry changed in place (a new
    /// revision, its counts, read) told so. Shown while the panel is open in an active window, what is new is read.
    fn show(mut self: Pin<&mut Self>, listing: &Listing) {
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
                    let _ = self.as_mut().begin_move_rows(&root, qt_int(from), qt_int(from), &root, qt_int(to));
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
        if self.marking {
            self.mark_shown_read();
        }
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
        app::get().inbox().dismiss_all(ids);
    }

    /// Put the entry at `row` on the clipboard: at once for a kept notice; for a ledger, once its files (or folders) are
    /// read off the window's thread.
    pub(crate) fn copy(&self, row: i32) {
        let Some(entry) = usize::try_from(row).ok().and_then(|row| self.rows.get(row)) else { return };
        let rules = pg_platform::native::PATH_RULES;
        let notice = entry.notice.clone();
        let Some(kind) = ledger_kind(entry.id) else {
            crate::clipboard::ffi::copy_text(&QString::from(&copy_words(&notice, rules, None)));
            return;
        };
        let notices = self.qt_thread();
        app::get().reading().session().copy_details(kind, COPIED_FILES, move |read| {
            let items = match read {
                Ok(failed) => Some(failed.items),
                Err(e) => {
                    pg_log::warn("inbox", &format!("the failed files to copy couldn't be read: {e}"));
                    None
                }
            };
            let words = copy_words(&notice, rules, items.as_ref());
            // On the Qt thread, which the clipboard is; fails only when the window is gone.
            let _ = notices.queue(move |_| crate::clipboard::ffi::copy_text(&QString::from(&words)));
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
        kind_code(LedgerKind::from_code(&code.to_string()))
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
    use pg_app::reexport::{FailedFile, FailedFolder, FolderFailure};
    use pg_model::ids::{FileId, RootId};
    use pg_model::notice::{FilePass, LedgerCounts};

    use super::*;

    fn entry(id: InboxEntryId, notice: Notice) -> InboxEntry {
        InboxEntry { id, notice, counts: None, created: 0, updated: 0, read: false, decode_error: None }
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
            entry(row(1), Notice::UnreadableFiles { pass: FilePass::Import, count: 2 }),
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
        assert_eq!(lines.next(), Some(text::notice_line(&notice, rules).as_str()));
        assert_eq!(lines.next(), Some(r"D:\p\a.jpg: not a JPEG"));
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
        assert_eq!(copy_words(&notice, rules, None), text::notice_line(&notice, rules));
    }
}
