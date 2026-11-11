//! What makes two files identical. A leaf below `core`: `Catalog`'s methods take it, and the compiler graph puts an
//! inherent method in its type's module, so a type they name has to be below `core` too.

/// What makes two files identical: their bytes, or their decoded pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Identity {
    /// The same file bytes (tier 0a).
    File,
    /// The same decoded pixels (tier 0b).
    Pixels,
}
