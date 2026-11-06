//! The duplicates' grouping over the tier-1 hashes: the PDQ index (multi-index hashing, `pdq_index`), the import's
//! pairing (`Pairer`: each new picture queried against the index, then inserted), and the union-find that groups the
//! pairs (`UnionFind`). Over the model's hashes only: the catalog hands the hashes in and stores the pairs found.

// A library reports through its results; printing is for the command line and the app.
#![cfg_attr(not(test), deny(clippy::print_stdout, clippy::print_stderr))]

pub mod pairer;
pub mod pdq_index;
pub mod union_find;

pub use pairer::Pairer;
pub use pdq_index::IndexFull;
pub use union_find::UnionFind;
