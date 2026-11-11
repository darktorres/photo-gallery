//! What the inbox reads of a ledger: its acknowledgement, its cheap state, its counts and the folders of the scans'
//! ledger. A leaf below `core`: `Catalog`'s methods take and return these, and the compiler graph puts an inherent
//! method in its type's module, so a type they name has to be below `core` too. Uses only `types`.

use pg_model::ids::RootId;
use pg_model::rules::Path as RulePath;

use crate::types::FolderFailure;

/// What the user saw of a ledger's entry (`ledger_ack`), by serial: the newest row shown when they last read it, and
/// when they dismissed it, up to which row. A ledger never acknowledged reads as the default: nothing seen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LedgerAck {
    /// The newest serial listed when it was last read or dismissed.
    pub seen_to: i64,
    /// The newest serial listed when it was dismissed; `None` while it isn't.
    pub dismissed_to: Option<i64>,
    /// Whether it was read since the rows it was read with (a later row makes it unread again).
    pub read: bool,
}

/// A ledger row as its state names it: its serial, and when it was first recorded (Unix seconds).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LedgerRow {
    pub serial: i64,
    pub first_seen: i64,
}

/// A ledger's state, from its newest and oldest rows alone (each one index seek: `Catalog::ledger_state`), and the
/// entry's rules (docs/NOTIFICATIONS.md) applied to it with its acknowledgement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LedgerState {
    /// The ledger has rows the inbox can show (for the files: of files present, and for the import not pending).
    pub live: bool,
    /// A row came after what the user last saw (`serial > seen_to`).
    pub new: bool,
    /// A row came after the dismissal (`serial > dismissed_to`); false while it isn't dismissed.
    pub since_dismissed: bool,
    /// The entry is listed: live, and not dismissed or with a row since.
    pub shown: bool,
    /// The entry is listed and not read: never read, or new since.
    pub unread: bool,
    /// The newest row, whose serial a mark or a dismissal covers up to, and the oldest.
    pub newest: Option<LedgerRow>,
    pub oldest: Option<LedgerRow>,
}

impl LedgerState {
    /// The state of a ledger whose newest and oldest live rows are these, acknowledged so.
    pub(crate) fn of(newest: Option<LedgerRow>, oldest: Option<LedgerRow>, ack: &LedgerAck) -> LedgerState {
        let newest_serial = newest.map(|r| r.serial);
        let live = newest.is_some();
        let new = newest_serial.is_some_and(|s| s > ack.seen_to);
        let since_dismissed = matches!((newest_serial, ack.dismissed_to), (Some(s), Some(d)) if s > d);
        let shown = live && (ack.dismissed_to.is_none() || since_dismissed);
        let unread = shown && (!ack.read || new);
        LedgerState { live, new, since_dismissed, shown, unread, newest, oldest }
    }
}

/// A ledger's counts, read only while the inbox is viewed (`count(*)` over its live rows): how many, how many came after
/// what the user last saw, and after the dismissal. A kept notice's counts are its repeats (`total`), with nothing new.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub total: u64,
    pub new: u64,
    pub since_dismissed: u64,
}

/// A folder of the scans' ledger, as the inbox's expanded entry lists it (`Catalog::failed_folders`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FailedFolder {
    /// The library folder that holds it.
    pub root: RootId,
    pub root_path: RulePath,
    /// The folder, exact.
    pub path: RulePath,
    pub failure: FolderFailure,
    pub error: String,
    /// When a walk first found it (Unix seconds).
    pub first_seen: i64,
    /// Whether it came after what the user last saw of the entry.
    pub new: bool,
}
