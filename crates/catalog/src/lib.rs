//! The catalog: SQLite in WAL mode, the source of truth for files, analyses, pairs and ratings.
//!
//! File identity is (volume serial, NTFS file ID), so moves and renames need no re-analysis. An analysis
//! row records what it was computed from: the size, mtime, catalog rotation and kind, and the pipeline version.
//! When any differ, the file is pending again (`files.pending`, kept by triggers). That is all the resumability
//! there is: every batch is one transaction, and the work list is an index.
//!
//! The thumbnails' work is tracked the same way: whatever a thumbnail depends on changing gives the file the next
//! number of a change sequence (`files.seq`, from `meta.seq`), so the thumbnail pass looks only at files changed
//! since it last looked (`pg_thumbcache::pending_in`).
//!
//! A module the docs call a leaf (below `core`) is one whose types `Catalog`'s methods take or return. The compiler
//! graph puts an inherent method in its type's module, so a type named in those signatures has to sit below `core` too.

#![forbid(unsafe_code)]

// The SQL macros are declared first: `macro_rules!` is textually scoped, so every module declared after
// this one can use them.
#[macro_use]
mod sql;

/// The rows a catalog write batch holds in the import and borders passes (one transaction per batch).
pub const WRITE_BATCH: usize = 500;

pub mod browse;
pub mod dups;
#[cfg(any(test, feature = "fixture"))]
pub mod fixture;
pub mod inbox;
pub mod ledger;
pub mod pair;
pub mod queue;

mod borders;
mod classify;
mod convert;
mod core;
mod current;
mod lease;
mod lifecycle;
mod listing;
mod lookup;
mod progress;
pub use progress::{FreeSpace, NoProgress, Phase, Progress};
mod rebuild;
mod roots;
mod transaction;
pub use transaction::{TxMark, attach_tx, tx_mark};
mod types;
mod work;
mod writers;

mod error;
pub use error::{Error, Result};
mod removal;
pub use removal::{Discarded, Driven, Marker};
mod schema;

pub use self::core::{BoundedCache, Catalog, Prepare, Reader, ScanLease, Tx};
pub use roots::FolderRowReader;
pub use transaction::{Patience, PatienceGuard, wait_for_writers};
pub use types::{
    CatalogCounts, CropSource, Folder, FolderRows, Listed, PairingMarks, PairingWork, PlaceholderWork, Prior, Rebuilt,
    Recorded, Root, SCAN_LEASE_SECS, ScannedFile, Skip, SkipReason, Versions, VolumeSet, WorkItem, WriteTarget,
};
// The writers are reached through `Tx`'s accessors, which name them in their signatures; no other crate names one.
pub use writers::{OwedHashes, PairsBatch};

#[cfg(test)]
mod test_support;
