//! What starting a library folder's removal found.

use alloc::collections::BTreeSet;

use crate::rules::Path as RulePath;

/// The lanes a removal parks: the volumes its folder's present files are on. `all` parks every lane, when a present file
/// has no volume, or when no file is present to tell the volumes by.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Volumes {
    /// The volumes of the folder's present files, as the lanes' sets have them.
    pub volumes: BTreeSet<u64>,
    /// Whether every lane is parked, whatever its volumes.
    pub all: bool,
}

/// What the catalog's `begin_removal` found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Begun {
    /// The folder left the library and its marker is made: the removal runs from here. `volumes` are the lanes the removal
    /// parks before it hides the folder's files, so no lane works on them from the moment the removal begins.
    Started {
        /// The folder that left the library.
        path: RulePath,
        /// The lanes the removal parks.
        volumes: Volumes,
    },
    /// A removal of this folder is already under way: the marker is there, and the folder is not in the library.
    Pending(RulePath),
    /// No library folder has that id.
    NotFound,
    /// The id names a library folder, but not the one the caller showed: the list changed since. Nothing was changed.
    Changed,
}
