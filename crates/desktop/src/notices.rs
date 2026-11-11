//! The window's inbox (InboxPanel.qml, the bell and the toast): what `pg_app::Inbox` lists, newest first, worded by
//! `pg_client::text`. `start` subscribes to the inbox; a thread hands what it publishes to the Qt thread in passes, one
//! queued at a time, as the library's changes are (`library.rs`): the latest listing, and the toasts that came with it.
//! The window never reads the catalog for the inbox. A mark or a dismissal changes the rows here at once and is queued
//! for the inbox's writer, whose next listing confirms it.

pub(crate) mod state;

use crate::notices_bridge::qobject;

use core::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QByteArray, QHash, QHashPair_i32_QByteArray, QModelIndex, QString, QVariant};
use pg_app::reexport::InboxEntryId;
use pg_app::{InboxEvent, Listing};
use pg_client::text::{self, Message, render};
use pg_model::notice::{Action, FilePass, Level, Notice};

use crate::app;
use crate::failed_files::{pass_code, pass_of};

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

/// The most failed files Copy puts on the clipboard for a ledger's entry.
const COPIED_FILES: usize = 1000;

/// The pass whose failure ledger the entry is, if it is one.
fn ledger_pass(id: InboxEntryId) -> Option<FilePass> {
    match id {
        InboxEntryId::Ledger { pass, .. } => Some(pass),
        InboxEntryId::Row { .. } => None,
    }
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
            ROLE_COUNT => QVariant::from(&qt_int(entry.count)),
            ROLE_READ => QVariant::from(&entry.read),
            // A ledger's entry lists its files when it is opened (`FailedFiles.show`).
            ROLE_PASS => QVariant::from(&pass_code(ledger_pass(entry.id))),
            ROLE_CAN_RETRY => QVariant::from(&notice.actions().contains(&Action::Retry)),
            ROLE_CAN_SHOW_FILES => QVariant::from(&notice.actions().contains(&Action::ShowFailedFiles)),
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

    fn show(mut self: Pin<&mut Self>, listing: &Listing) {
        self.as_mut().begin_reset_model();
        self.as_mut().rust_mut().rows = listing.entries.clone();
        self.as_mut().end_reset_model();
        self.as_mut().set_unread_count(qt_int(listing.unread));
    }

    fn set_unread_count(mut self: Pin<&mut Self>, unread: i32) {
        if self.unread != unread {
            self.as_mut().rust_mut().unread = unread;
            self.unread_changed();
        }
    }

    fn set_toast_shown(mut self: Pin<&mut Self>, words: QString, level: i32) {
        self.as_mut().rust_mut().toast = words;
        self.as_mut().rust_mut().toast_level = level;
        self.as_mut().toast_level_changed();
        self.toast_changed();
    }

    pub(crate) fn mark_all_read(mut self: Pin<&mut Self>) {
        if self.unread == 0 {
            return;
        }
        self.as_mut().begin_reset_model();
        for entry in &mut self.as_mut().rust_mut().rows {
            entry.read = true;
        }
        self.as_mut().end_reset_model();
        self.as_mut().set_unread_count(0);
        app::get().inbox().mark_all_read();
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
        app::get().inbox().dismiss(entry.id);
    }

    pub(crate) fn dismiss_all(mut self: Pin<&mut Self>) {
        self.as_mut().begin_reset_model();
        self.as_mut().rust_mut().rows.clear();
        self.as_mut().end_reset_model();
        self.as_mut().set_unread_count(0);
        app::get().inbox().dismiss_all();
    }

    pub(crate) fn copy(&self, row: i32) {
        let Some(entry) = usize::try_from(row).ok().and_then(|row| self.rows.get(row)) else { return };
        let rules = pg_platform::native::PATH_RULES;
        let mut words = text::notice_line(&entry.notice, rules);
        // A ledger's entry: its files, one a line.
        if let Some(pass) = ledger_pass(entry.id) {
            match app::get().catalogs().failed_files(pass, COPIED_FILES) {
                Ok(files) => {
                    for file in files {
                        words.push_str(&format!("\n{}: {}", rules.display(&file.path), file.error));
                    }
                }
                Err(e) => pg_log::warn("inbox", &format!("the failed files couldn't be read: {e}")),
            }
        }
        crate::clipboard::ffi::copy_text(&QString::from(&words));
    }

    /// The row of a pass's entry (a `pass` code), -1 when the inbox lists none.
    pub(crate) fn row_of_pass(&self, pass: i32) -> i32 {
        let Some(pass) = pass_of(pass) else { return -1 };
        self.rows.iter().position(|e| ledger_pass(e.id) == Some(pass)).map_or(-1, qt_int)
    }

    /// Try the failed files of the entry at `row` again (a pass's ledger with Retry), then wake the job to make them.
    /// The entry goes: the files that still fail make it anew.
    pub(crate) fn retry(mut self: Pin<&mut Self>, row: i32) {
        let Some(at) = usize::try_from(row).ok().filter(|&at| at < self.rows.len()) else { return };
        let Some(pass) = self.rows.get(at).and_then(|e| ledger_pass(e.id)) else { return };
        self.as_mut().dismiss(row);
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
