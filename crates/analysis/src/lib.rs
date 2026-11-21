//! What the import computes from a file: its kind, size, orientation, frames and dates from the header and the
//! container (no decode), and the duplicate hashes, the border detector and the PDQ hasher the later passes use. No
//! catalog and no file writes here, and no UI.

pub mod analyze;
pub mod borders;
pub mod pdq;
