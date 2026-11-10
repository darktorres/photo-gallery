//! The window's log view (dev mode, LogPanel.qml): the records of `gallery.log` and this run's, oldest first, following
//! the new ones as they are logged. `start` subscribes to the log first (`pg_log::subscribe`), then reads the files on a
//! thread of its own and merges the two (none missed, none twice); from then on each new record is handed to the Qt
//! thread in passes, one queued at a time, as the library's changes are (`library.rs`). At most `pg_log::RING_RECORDS`
//! rows are kept: the oldest go as new ones come.

pub(crate) mod state;

use crate::log_model_bridge::qobject;

use core::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QByteArray, QHash, QHashPair_i32_QByteArray, QModelIndex, QString, QVariant};
use pg_model::notice::Level;

const ROLE_TIME: i32 = 256;
const ROLE_LEVEL: i32 = 257;
const ROLE_TARGET: i32 = 258;
const ROLE_MESSAGE: i32 = 259;
const ROLE_DETAIL: i32 = 260;

/// The records logged that the Qt thread hasn't shown, and whether a pass is queued for them.
#[derive(Default)]
struct Pending {
    records: Vec<pg_log::Record>,
    queued: bool,
}

fn lock(pending: &Mutex<Pending>) -> MutexGuard<'_, Pending> {
    pending.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The `level` role: 0 info, 1 warning, 2 error (LogPanel.qml colours them).
fn level_code(level: Level) -> i32 {
    match level {
        Level::Info => 0,
        Level::Warning => 1,
        Level::Error => 2,
    }
}

impl qobject::LogModel {
    pub(crate) fn data(&self, index: &QModelIndex, role: i32) -> QVariant {
        let Some(record) = usize::try_from(index.row()).ok().and_then(|row| self.rows.get(row)) else {
            return QVariant::default();
        };
        match role {
            ROLE_TIME => QVariant::from(&QString::from(&record.local)),
            ROLE_LEVEL => QVariant::from(&level_code(record.level)),
            ROLE_TARGET => QVariant::from(&QString::from(&record.target)),
            ROLE_MESSAGE => QVariant::from(&QString::from(&record.message)),
            ROLE_DETAIL => QVariant::from(&QString::from(record.detail.as_deref().unwrap_or_default())),
            _ => QVariant::default(),
        }
    }

    pub(crate) fn role_names(&self) -> QHash<QHashPair_i32_QByteArray> {
        let mut roles = QHash::<QHashPair_i32_QByteArray>::default();
        for (role, name) in [
            (ROLE_TIME, "time"),
            (ROLE_LEVEL, "level"),
            (ROLE_TARGET, "target"),
            (ROLE_MESSAGE, "message"),
            (ROLE_DETAIL, "detail"),
        ] {
            roles.insert(role, QByteArray::from(name));
        }
        roles
    }

    pub(crate) fn row_count(&self, _parent: &QModelIndex) -> i32 {
        i32::try_from(self.rows.len()).unwrap_or(i32::MAX)
    }

    pub(crate) fn start(mut self: Pin<&mut Self>) {
        if self.started {
            return;
        }
        self.as_mut().rust_mut().started = true;
        // Subscribed before the files are read, so a record logged meanwhile is in the feed (and left out of the files'
        // part by the merge).
        let feed = pg_log::subscribe();
        let path = pg_log::path();
        let thread = self.qt_thread();
        let reader = std::thread::Builder::new().name("log view".into()).spawn(move || {
            let from_files = path.map(|p| pg_log::read(&p, pg_log::RING_RECORDS)).unwrap_or_default();
            let mut all = feed.merge(from_files);
            let extra = all.len().saturating_sub(pg_log::RING_RECORDS);
            all.drain(..extra);
            // Fails only when the window is gone: the reader ends with it.
            if thread.queue(move |model| model.show_all(all)).is_err() {
                return;
            }
            let pending: Arc<Mutex<Pending>> = Arc::default();
            while let Ok(record) = feed.new.recv() {
                let queue = {
                    let mut pending = lock(&pending);
                    pending.records.push(record);
                    !std::mem::replace(&mut pending.queued, true)
                };
                if queue {
                    let pass = Arc::clone(&pending);
                    if thread.queue(move |model| model.append(pass)).is_err() {
                        break;
                    }
                }
            }
        });
        if let Err(e) = reader {
            pg_log::warn("log", &format!("the log view's reader didn't start: {e}"));
        }
    }

    fn show_all(mut self: Pin<&mut Self>, records: Vec<pg_log::Record>) {
        self.as_mut().begin_reset_model();
        self.as_mut().rust_mut().rows = records.into();
        self.as_mut().end_reset_model();
        self.as_mut().rust_mut().ready = true;
        self.ready_changed();
    }

    /// The records logged since the last pass, added at the end; the oldest rows go past the cap.
    fn append(mut self: Pin<&mut Self>, pending: Arc<Mutex<Pending>>) {
        let records = {
            let mut pending = lock(&pending);
            pending.queued = false;
            std::mem::take(&mut pending.records)
        };
        if records.is_empty() {
            return;
        }
        let first = self.rows.len();
        let last = first + records.len() - 1;
        let root = QModelIndex::default();
        self.as_mut().begin_insert_rows(
            &root,
            i32::try_from(first).unwrap_or(i32::MAX),
            i32::try_from(last).unwrap_or(i32::MAX),
        );
        self.as_mut().rust_mut().rows.extend(records);
        self.as_mut().end_insert_rows();
        let extra = self.rows.len().saturating_sub(pg_log::RING_RECORDS);
        if extra > 0 {
            self.as_mut().begin_remove_rows(&root, 0, i32::try_from(extra - 1).unwrap_or(i32::MAX));
            self.as_mut().rust_mut().rows.drain(..extra);
            self.as_mut().end_remove_rows();
        }
    }
}
