//! What makes two files identical. A leaf below `core` (the rule is in the crate's docs).

/// What makes two files identical: their bytes, or their decoded pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Identity {
    /// The same file bytes (tier 0a).
    File,
    /// The same decoded pixels (tier 0b).
    Pixels,
}
