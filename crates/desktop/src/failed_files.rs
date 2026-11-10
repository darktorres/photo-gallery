//! The files of an inbox entry for a pass's failure ledger (InboxPanel.qml, under the entry): the newest of the files the
//! pass failed on, each with its folder and name, its error and when it first failed, and its id for Reveal. Read from
//! the catalog when the entry is opened (`pg_app::Catalogs::failed_files`), the one read of the inbox's the window
//! makes: the listing has only the entries' counts.

pub(crate) mod state;

use crate::failed_files_bridge::qobject;

use core::pin::Pin;

use cxx_qt::CxxQtType;
use cxx_qt_lib::{QByteArray, QHash, QHashPair_i32_QByteArray, QModelIndex, QString, QVariant};
use pg_app::reexport::InboxEntryId;
use pg_model::notice::FilePass;

use crate::app;

/// The most files an entry lists.
const SHOWN: usize = 200;

const ROLE_PATH: i32 = 256;
const ROLE_ERROR: i32 = 257;
const ROLE_FILE_ID: i32 = 258;
const ROLE_FIRST_SEEN: i32 = 259;
const ROLE_NEW: i32 = 260;
const ROLE_RETRIED: i32 = 261;

/// A pass's code in QML (a `Notices` row's `pass`, `Library.failedPass`): its place in [`FilePass::ALL`], -1 for none.
pub(crate) fn pass_code(pass: Option<FilePass>) -> i32 {
    pass.and_then(|p| FilePass::ALL.iter().position(|&q| q == p)).and_then(|i| i32::try_from(i).ok()).unwrap_or(-1)
}

/// The pass of a QML code ([`pass_code`]).
pub(crate) fn pass_of(code: i32) -> Option<FilePass> {
    usize::try_from(code).ok().and_then(|i| FilePass::ALL.get(i)).copied()
}

impl qobject::FailedFiles {
    pub(crate) fn data(&self, index: &QModelIndex, role: i32) -> QVariant {
        let Some(file) = usize::try_from(index.row()).ok().and_then(|row| self.rows.get(row)) else {
            return QVariant::default();
        };
        let rules = pg_platform::native::PATH_RULES;
        match role {
            ROLE_PATH => QVariant::from(&QString::from(&rules.display(&file.path))),
            ROLE_ERROR => QVariant::from(&QString::from(&file.error)),
            // A number QML holds exactly (ids are far below 2^53).
            ROLE_FILE_ID => QVariant::from(&pg_model::num::signed(file.file_id.get())),
            ROLE_FIRST_SEEN => QVariant::from(&pg_model::num::signed(file.first_seen)),
            ROLE_NEW => QVariant::from(&file.new),
            // The pass tries it again by itself; else it waits for a change, or Retry.
            ROLE_RETRIED => QVariant::from(&file.retry),
            _ => QVariant::default(),
        }
    }

    pub(crate) fn role_names(&self) -> QHash<QHashPair_i32_QByteArray> {
        let mut roles = QHash::<QHashPair_i32_QByteArray>::default();
        for (role, name) in [
            (ROLE_PATH, "path"),
            (ROLE_ERROR, "error"),
            (ROLE_FILE_ID, "fileId"),
            (ROLE_FIRST_SEEN, "firstSeen"),
            (ROLE_NEW, "isNew"),
            (ROLE_RETRIED, "retried"),
        ] {
            roles.insert(role, QByteArray::from(name));
        }
        roles
    }

    pub(crate) fn row_count(&self, _parent: &QModelIndex) -> i32 {
        i32::try_from(self.rows.len()).unwrap_or(i32::MAX)
    }

    pub(crate) fn show(mut self: Pin<&mut Self>, pass: i32) {
        let app = app::get();
        let pass = pass_of(pass);
        let rows = match pass {
            Some(pass) => app.catalogs().failed_files(pass, SHOWN).unwrap_or_else(|e| {
                pg_log::warn("inbox", &format!("the failed files couldn't be read: {e}"));
                Vec::new()
            }),
            None => Vec::new(),
        };
        let total = app
            .inbox()
            .listing()
            .entries
            .iter()
            .find(|e| matches!(e.id, InboxEntryId::Ledger { pass: p, .. } if Some(p) == pass))
            .map_or(rows.len(), |e| usize::try_from(e.count).unwrap_or(usize::MAX));
        self.as_mut().begin_reset_model();
        self.as_mut().rust_mut().rows = rows;
        self.as_mut().end_reset_model();
        let total = i32::try_from(total).unwrap_or(i32::MAX);
        if self.total != total {
            self.as_mut().rust_mut().total = total;
            self.total_changed();
        }
    }
}
