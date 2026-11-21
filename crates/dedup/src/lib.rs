//! The duplicates' grouping over the tier-1 hashes: the all-pairs join that finds the pairs (`join`, run by the
//! catalog's pairing pass after the import), and the union-find that groups them (`UnionFind`). Over the model's hashes
//! only: the catalog hands the hashes in and stores the pairs found.

pub mod join;
pub mod union_find;

pub use union_find::UnionFind;
