//! What a scan of a root saw, for its pairing. A leaf below `core`: `Catalog`'s methods take it, and the compiler graph
//! puts an inherent method in its type's module, so a type they name has to be below `core` too.

use crate::types::Skip;

/// What a scan of a root saw, for its pairing.
#[derive(Clone, Copy, Debug)]
pub struct Coverage<'a> {
    /// The walk finished: it was not stopped. Tier 2, the replacements and the clearing need it.
    pub complete: bool,
    /// The folders the walk did not list, whatever the reason (those it couldn't read, the mounts it refuses and the
    /// reparse folders it does not follow). Their subtrees were not seen, so their rows are left out of every tier.
    pub skipped: &'a [Skip],
}
