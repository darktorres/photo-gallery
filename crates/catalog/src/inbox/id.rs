//! What names an entry of the inbox. A leaf below `core`: `Catalog`'s methods take it, and the compiler graph puts an
//! inherent method in its type's module, so a type they name has to be below `core` too.

use pg_model::notice::LedgerKind;

/// An entry of the inbox, as a listing names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EntryId {
    /// A kept notice: its row's id and `uid` (`meta.seq` when it was made), at its revision `rev` (which rises when a
    /// state's data changes). Ids are reused, so the pair is what names the row: a mark or a dismissal for a row since
    /// gone finds none. A mark names the revision it was made for, so one sent for a revision never lands on the next
    /// and swallows its news.
    Row { id: i64, uid: i64, rev: i64 },
    /// A ledger, as listed: up to its newest row (`ledger::Summary::newest`). A mark or a dismissal covers the rows
    /// listed, and leaves the later ones new.
    Ledger { kind: LedgerKind, seen: i64 },
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
