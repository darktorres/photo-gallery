//! The window's log view (dev mode, LogPanel.qml): the records of `gallery.log` and this run's, oldest first, following
//! the new ones as they are logged. `start` subscribes to the log first (`pg_log::subscribe`), then reads the files on a
//! thread of its own and merges the two (none missed, none twice); from then on each new record is handed to the Qt
//! thread in passes, one queued at a time, as the library's changes are (`library.rs`). At most `pg_log::RING_RECORDS`
//! rows are kept: the oldest go as new ones come.
//!
//! Each row has a number of its own (the `seq` role): its place among every record the view was given, from 0, so a
//! row keeps it as the rows before it are trimmed, and the panel's selection follows it. A record's own `seq` would not
//! do: it counts within its session only, and an older build's plain line has none.
//!
//! With `--log-view=<path>` (the screenshots' fixed log) the view shows that file, read with the log's reader, and
//! follows nothing: the app's own logging is unchanged.
//!
//! ## Dev mode and the log view
//!
//! Dev mode shows the log view's button in the status bar. **Ctrl+Shift+D** turns it on and off and saves it in the
//! preferences (`dev_mode`); `--dev=1` or `PG_DEV=1` turns it on for one run without saving it, and leaves the preferences
//! file as it was. **Ctrl+Shift+L** opens and closes the log view in dev mode, and **Ctrl+Shift+N** the inbox.
//!
//! The shortcuts are Qt `Shortcut`s (`GlobalShortcuts.qml`), which Qt matches before the focused item's keys, so they work
//! from the grid, the viewer and a text field. What a shortcut does is the controller's (`pg_client::controller`): while a
//! dialog or a menu is open, it is that panel's and the shortcut does nothing; the log view's key closes it when it is open,
//! and opens it otherwise (closing the inbox first, and the inbox's the log). Each modal panel reports its opening and
//! closing (`Shell.panelOpened`, `panelClosed`, by its `PanelKind.qml` code), so the controller knows which are open. The
//! open panels are an ordered set (`Panels.modals`): opening a kind already open moves it to the top, and closing removes it
//! wherever it is, so it holds each kind at most once.
//!
//! The log view (`LogPanel.qml`, over `LogModel`) shows this run's records and the files', oldest first. It follows new
//! ones while it is at the end, and stops following when the user scrolls away; the selection is by record (`seq`), kept
//! when the oldest rows are trimmed, and cleared if its own row was. A row is one line; a click shows its whole message and
//! detail below. Escape or Close dismisses it, and Tab stays in it. `--log-view=<file>` shows that file's records instead,
//! read with the `pg_log` reader and subscribed to nothing (the `log` screenshot); the app's own logging is unchanged.

use crate::bridges::log_model_bridge::qobject;

use core::pin::Pin;
use std::sync::{Arc, Mutex};

use crate::pump::Pending;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QByteArray, QHash, QHashPair_i32_QByteArray, QModelIndex, QString, QVariant};

const ROLE_TIME: i32 = 256;
const ROLE_LEVEL: i32 = 257;
const ROLE_TARGET: i32 = 258;
const ROLE_MESSAGE: i32 = 259;
const ROLE_DETAIL: i32 = 260;
const ROLE_SEQ: i32 = 261;

/// The warning for a reader thread that couldn't be spawned.
fn reader_didnt_start(e: &std::io::Error) {
    pg_log::warn("log", &format!("the log view's reader didn't start: {e:#}"));
}

impl qobject::LogModel {
    pub(crate) fn data(&self, index: &QModelIndex, role: i32) -> QVariant {
        let Ok(row) = usize::try_from(index.row()) else {
            return QVariant::default();
        };
        let Some(record) = self.rows.get(row) else {
            return QVariant::default();
        };
        match role {
            ROLE_TIME => QVariant::from(&QString::from(&record.local)),
            ROLE_LEVEL => QVariant::from(&record.level.code()),
            ROLE_TARGET => QVariant::from(&QString::from(&record.target)),
            ROLE_MESSAGE => QVariant::from(&QString::from(&record.message)),
            ROLE_DETAIL => QVariant::from(&QString::from(record.detail.as_deref().unwrap_or_default())),
            // A number QML holds exactly (far below 2^53).
            ROLE_SEQ => QVariant::from(&pg_model::num::size(self.seq_of(row))),
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
            (ROLE_SEQ, "seq"),
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
        let thread = self.qt_thread();
        if let Some(path) = crate::command_line::log_view() {
            let path = path.to_path_buf();
            let reader = std::thread::Builder::new().name("log view".into()).spawn(move || {
                let records = pg_log::read(&path, pg_log::RING_RECORDS);
                let _ = thread.queue(move |model| model.show_all(records));
            });
            if let Err(e) = reader {
                reader_didnt_start(&e);
            }
            return;
        }
        // Subscribed before the files are read, so a record logged meanwhile is in the feed (and left out of the files'
        // part by the merge).
        let feed = pg_log::subscribe();
        let path = pg_log::path();
        let reader = std::thread::Builder::new().name("log view".into()).spawn(move || {
            let from_files = path.map(|p| pg_log::read(&p, pg_log::RING_RECORDS)).unwrap_or_default();
            let mut all = feed.merge(from_files);
            let extra = all.len().saturating_sub(pg_log::RING_RECORDS);
            all.drain(..extra);
            // Fails only when the window is gone: the reader ends with it.
            if thread.queue(move |model| model.show_all(all)).is_err() {
                return;
            }
            // The records logged from here on are gathered by the pump and appended after the rows just queued.
            let pump = crate::pump::start(
                "log changes",
                thread,
                move || feed.new.recv().ok(),
                |records: &mut Vec<pg_log::Record>, record: pg_log::Record| records.push(record),
                |model, pending| model.append(pending),
            );
            if let Err(e) = pump {
                pg_log::warn("log", &format!("the log view's pump didn't start: {e:#}"));
            }
        });
        if let Err(e) = reader {
            reader_didnt_start(&e);
        }
    }

    fn show_all(mut self: Pin<&mut Self>, records: Vec<pg_log::Record>) {
        self.as_mut().begin_reset_model();
        self.as_mut().rust_mut().reset(records);
        self.as_mut().end_reset_model();
        self.as_mut().rust_mut().ready = true;
        self.as_mut().ready_changed();
        // The numbers start again, so the selection of the view before means nothing: it is cleared.
        self.as_mut().rust_mut().selected = None;
        self.as_mut().refresh_selection();
    }

    /// The records logged since the last pass, added at the end; the oldest rows go past the cap.
    fn append(mut self: Pin<&mut Self>, pending: Arc<Mutex<Pending<Vec<pg_log::Record>>>>) {
        let records = Pending::take(&pending);
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
        let extra = self.over_cap();
        if extra > 0 {
            self.as_mut().begin_remove_rows(&root, 0, i32::try_from(extra - 1).unwrap_or(i32::MAX));
            self.as_mut().rust_mut().drop_oldest(extra);
            self.as_mut().end_remove_rows();
        }
        self.as_mut().refresh_selection();
    }

    /// Select the row at `row` (none for a row past the list). The panel shows its message, with its detail below.
    pub(crate) fn select(mut self: Pin<&mut Self>, row: i32) {
        let selected = usize::try_from(row).ok().filter(|&at| at < self.rows.len()).map(|at| self.seq_of(at));
        self.as_mut().rust_mut().selected = selected;
        self.as_mut().refresh_selection();
    }

    /// Clear the selection.
    pub(crate) fn unselect(mut self: Pin<&mut Self>) {
        self.as_mut().rust_mut().selected = None;
        self.as_mut().refresh_selection();
    }

    /// The selection as the panel reads it. It is held by number, so it follows its row as the oldest rows go; it is
    /// cleared when its row is gone.
    fn refresh_selection(mut self: Pin<&mut Self>) {
        let at = self
            .selected
            .and_then(|seq| seq.checked_sub(self.first))
            .and_then(|at| usize::try_from(at).ok())
            .filter(|&at| at < self.rows.len());
        if at.is_none() && self.selected.is_some() {
            self.as_mut().rust_mut().selected = None;
        }
        let (row, seq, text) = match at.and_then(|at| self.rows.get(at).map(|record| (at, record))) {
            Some((at, record)) => {
                let text = match record.detail.as_deref().filter(|detail| !detail.is_empty()) {
                    Some(detail) => format!("{}\n\n{detail}", record.message),
                    None => record.message.clone(),
                };
                let seq = self.selected.map_or(-1.0, pg_model::num::size);
                (i32::try_from(at).unwrap_or(-1), seq, QString::from(&text))
            }
            None => (-1, -1.0, QString::default()),
        };
        self.as_mut().rust_mut().selected_row = row;
        self.as_mut().selected_row_changed();
        self.as_mut().rust_mut().selected_seq = seq;
        self.as_mut().selected_seq_changed();
        self.as_mut().rust_mut().selected_text = text;
        self.as_mut().selected_text_changed();
    }
}

#[cfg(test)]
mod tests {
    use pg_model::notice::Level;

    #[test]
    fn each_level_has_the_code_the_panel_colours() {
        assert_eq!(Level::Info.code(), 0);
        assert_eq!(Level::Warning.code(), 1);
        assert_eq!(Level::Error.code(), 2);
    }
}
