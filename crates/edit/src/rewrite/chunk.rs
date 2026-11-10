//! PNG and WebP chunks: one made, and one's bytes in its file.

use anyhow::{Context, Result};
use pg_container::Chunk;

/// A PNG chunk of `kind` holding `data`; an error when `data` is longer than a chunk's length field counts.
pub(crate) fn png_chunk(kind: &[u8; 4], data: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(data.len() + 12);
    out.extend_from_slice(&u32::try_from(data.len())?.to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    // The CRC covers the type and the data.
    let mut crc = crc32fast::Hasher::new();
    crc.update(kind);
    crc.update(data);
    out.extend_from_slice(&crc.finalize().to_be_bytes());
    Ok(out)
}

/// A PNG or WebP chunk's bytes in the file it was read from, header and CRC or padding included.
pub(super) fn chunk_bytes<'a>(bytes: &'a [u8], c: &Chunk<'_>) -> Result<&'a [u8]> {
    bytes.get(c.at..c.at + c.len).context("a chunk outside its file")
}
