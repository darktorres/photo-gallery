//! The import's analysis of a file, from its decoded pixels and its facts: the analysis record's hashes (BLAKE3 of the
//! bytes, XXH3 of the pixels, the PDQ hash, the ThumbHash), the border detector, and the PDQ hasher. No catalog and no
//! file writes here, and no UI.

// A library reports through its results; printing is for the command line and the app.
#![cfg_attr(not(test), deny(clippy::print_stdout, clippy::print_stderr))]

pub mod analyze;
pub mod borders;
pub mod pdq;
