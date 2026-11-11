//! PNG and WebP chunks: one made (`pg_container::png_chunk`, its length checked), and one's bytes in its file; and
//! whether a file is animated, by the container's rule (the metadata's frame count applies the same one).

use anyhow::{Context, Result};
use pg_container::{Chunk, PngAnimation, WebpAnimation, png_chunks, webp_chunks, webp_lossless};

/// A PNG chunk of `kind` holding `data`; an error when `data` is longer than a chunk's length field counts.
pub(crate) fn png_chunk(kind: &[u8; 4], data: &[u8]) -> Result<Vec<u8>> {
    pg_container::png_chunk(kind, data).context("a PNG chunk longer than its length field counts")
}

/// A PNG or WebP chunk's bytes in the file it was read from, header and CRC or padding included.
pub(super) fn chunk_bytes<'a>(bytes: &'a [u8], c: &Chunk<'_>) -> Result<&'a [u8]> {
    bytes.get(c.at..c.at + c.len).context("a chunk outside its file")
}

/// Whether a PNG is an animation ([`PngAnimation`]: an `acTL` before its image data).
pub(crate) fn png_animated(bytes: &[u8]) -> bool {
    let mut animation = PngAnimation::default();
    png_chunks(bytes).for_each(|c| animation.see(&c));
    animation.animated()
}

/// Whether a WebP is an animation ([`WebpAnimation`]: the `VP8X` chunk's flag).
pub(crate) fn webp_animated(bytes: &[u8]) -> bool {
    let mut animation = WebpAnimation::default();
    webp_chunks(bytes).for_each(|c| animation.see(&c));
    animation.animated()
}

/// Whether a WebP's image is lossless (`VP8L`).
pub(crate) fn webp_is_lossless(bytes: &[u8]) -> bool {
    webp_chunks(bytes).any(|c| webp_lossless(&c))
}
