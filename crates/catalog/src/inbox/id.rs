//! What names an entry of the inbox. A leaf below `core`: `Catalog`'s methods take it, and the compiler graph puts an
//! inherent method in its type's module, so a type they name has to be below `core` too.

use pg_model::notice::FilePass;

/// An entry of the inbox, as a listing names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EntryId {
    /// A kept notice: its row's id and `uid` (`meta.seq` when it was made). Ids are reused, so the pair is what names
    /// it: a mark or a dismissal for a row since gone finds none.
    Row { id: i64, uid: i64 },
    /// A pass's failure ledger, as listed: up to its newest failure (`ledger::Summary::newest`). A mark or a dismissal
    /// covers the failures listed, and leaves the later ones new.
    Ledger { pass: FilePass, seen: i64 },
}

impl EntryId {
    /// The entry it names, whatever a ledger was listed up to: what tells whether two listings list the same entry.
    pub fn entry(self) -> EntryId {
        match self {
            EntryId::Ledger { pass, .. } => EntryId::Ledger { pass, seen: 0 },
            row @ EntryId::Row { .. } => row,
        }
    }
}
