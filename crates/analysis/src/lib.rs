//! What the import computes from a file: its kind, size, orientation, frames and dates from the header and the
//! container (no decode), and the duplicate hashes, the border detector and the PDQ hasher the later passes use. No
//! catalog and no file writes here, and no UI.

// A library reports through its results; printing is for the command line and the app.
#![cfg_attr(not(test), deny(clippy::print_stdout, clippy::print_stderr))]

pub mod analyze;
pub mod borders;
pub mod pdq;

/// Whether `bytes` (a prefix of a file, or the file) holds a JPEG's whole header, so the import can read it from a
/// prefix ([`pg_metadata::jpeg_header_complete`]). The import reads through this crate, not the metadata one.
pub fn jpeg_header_complete(bytes: &[u8]) -> bool {
    pg_metadata::jpeg_header_complete(bytes)
}
