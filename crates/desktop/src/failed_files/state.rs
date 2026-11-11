//! The state of the `FailedFiles` QObject: the bridge's backing struct (what the expanded ledger entry lists).

use cxx_qt_lib::QString;
use pg_app::reexport::{FailedFile, FailedFolder};
use pg_model::notice::LedgerKind;

/// One row of the list: a pass's failed file, or a folder the scans couldn't take.
pub(crate) enum Shown {
    File(FailedFile),
    Folder(FailedFolder),
}

#[derive(Default)]
#[expect(
    unreachable_pub,
    reason = "the bridge\'s generated code names it in a public interface (E0446), so it is `pub`"
)]
pub struct FailedFilesRust {
    pub(crate) total: i32,
    pub(crate) loading: bool,
    pub(crate) error: QString,
    /// The ledger shown (`show`), `None` for none.
    pub(super) kind: Option<LedgerKind>,
    /// The rows shown, newest first (a pass's files) or by library folder (the scans' folders).
    pub(super) rows: Vec<Shown>,
    /// The number of the last `show`: a delivery for an earlier one is dropped.
    pub(super) asked: u64,
}
