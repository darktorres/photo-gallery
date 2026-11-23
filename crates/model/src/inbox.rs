//! The inbox's entries as a listing gives them: a kept notice, or a ledger of failures, with its counts.

use alloc::string::String;

use crate::notice::{LedgerKind, Notice};

/// An entry of the inbox, as a listing names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum InboxEntryId {
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
    /// A ledger, as listed: up to its newest row. A mark or a dismissal covers the rows listed, and leaves the later
    /// ones new.
    Ledger {
        /// The ledger the rows are in.
        kind: LedgerKind,
        /// The newest row the ledger listed.
        seen: i64,
    },
}

impl InboxEntryId {
    /// The entry it names, whatever revision a row was listed at and whatever a ledger was listed up to: what tells
    /// whether two listings list the same entry (a new revision is the entry changed, not another entry).
    pub fn entry(self) -> InboxEntryId {
        match self {
            InboxEntryId::Ledger { kind, .. } => InboxEntryId::Ledger { kind, seen: 0 },
            InboxEntryId::Row { id, uid, .. } => InboxEntryId::Row { id, uid, rev: 0 },
        }
    }
}

/// A ledger's counts, read only while the inbox is viewed (`count(*)` over its live rows): how many, how many came
/// after what the user last saw, and after the dismissal. A kept notice's counts are its repeats (`total`), with nothing
/// new.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InboxCounts {
    /// The rows in all.
    pub total: u64,
    /// The rows not yet read.
    pub new: u64,
    /// The rows since the ledger was last dismissed.
    pub since_dismissed: u64,
}

/// An entry as the inbox lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InboxEntry {
    /// The entry's id: a row's or a ledger's (see [`InboxEntryId`]).
    pub id: InboxEntryId,
    /// What the entry says, as the notice this build makes of its kind.
    pub notice: Notice,
    /// A row's: how many times it was posted (a state's repeats, in `total`). A ledger's: read only while the inbox is
    /// viewed, `None` otherwise (nothing renders a count then).
    pub counts: Option<InboxCounts>,
    /// When it was first posted, and last (Unix seconds).
    pub created: i64,
    /// When it was last changed (Unix seconds).
    pub updated: i64,
    /// Whether the user has read it.
    pub read: bool,
    /// Why a stored row of a kind this build knows couldn't be read (it then reads as [`Notice::Unknown`]): a bad
    /// payload, which the inbox logs. `None` for a row read, and for a retired or unknown kind (expected).
    pub decode_error: Option<String>,
    /// A ledger's acknowledgement mark as listed (the ledger's `seen_to`); `None` for a row.
    pub seen_to: Option<i64>,
}
