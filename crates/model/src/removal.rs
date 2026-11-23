//! What starting a library folder's removal found.

use crate::rules::Path as RulePath;

/// What the catalog's `begin_removal` found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Begun {
    /// The folder left the library and its marker is made: the removal runs from here.
    Started(RulePath),
    /// A removal of this folder is already under way: the marker is there, and the folder is not in the library.
    Pending(RulePath),
    /// No library folder has that id.
    NotFound,
    /// The id names a library folder, but not the one the caller showed: the list changed since. Nothing was changed.
    Changed,
}
