//! What names an entry of the inbox. A leaf below `core` (the rule is in the crate's docs).

use pg_model::notice::LedgerKind;

/// An entry of the inbox, as a listing names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EntryId {
    /// A kept notice: its row's id and `uid` (`meta.seq` when it was made), at its revision `rev` (which rises when a
    /// state's data changes). Ids are reused, so the pair is what names the row: a mark or a dismissal for a row since
    /// gone finds none. A mark names the revision it was made for, so one sent for a revision never lands on the next
    /// and swallows its news.
    Row {
        /// The row's id in the inbox.
        id: i64,
        /// The row's unique id, which a stale mark can't match.
        uid: i64,
        /// The row's revision when the mark was made.
        rev: i64,
    },
    /// A ledger, as listed: up to its newest row (`ledger::Summary::newest`). A mark or a dismissal covers the rows
    /// listed, and leaves the later ones new.
    Ledger {
        /// The ledger the rows are in.
        kind: LedgerKind,
        /// The newest row the ledger listed.
        seen: i64,
    },
}

impl EntryId {
    /// The entry it names, whatever revision a row was listed at and whatever a ledger was listed up to: what tells
    /// whether two listings list the same entry (a new revision is the entry changed, not another entry).
    pub fn entry(self) -> EntryId {
        match self {
            EntryId::Ledger { kind, .. } => EntryId::Ledger { kind, seen: 0 },
            EntryId::Row { id, uid, .. } => EntryId::Row { id, uid, rev: 0 },
        }
    }
}
