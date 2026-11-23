//! What the inbox reads of a ledger: its acknowledgement, its cheap state, its counts and the folders of the scans'
//! ledger. A leaf below `core` (the rule is in the crate's docs). Its counts and folders are the model's
//! (`pg_model::inbox::InboxCounts`, `pg_model::ledger::FailedFolder`).

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
    /// The row's serial: its place in the ledger, which only grows.
    pub serial: i64,
    /// When the row was first seen (Unix seconds).
    pub first_seen: i64,
}

/// A ledger's state, from its newest and oldest rows alone (each one index seek: `Catalog::ledger_state`), and the
/// entry's rules (see the docs in `ledger.rs`) applied to it with its acknowledgement.
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
    /// The oldest row.
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
