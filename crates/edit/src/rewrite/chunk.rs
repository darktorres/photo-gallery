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

/// What a WebP is, by its chunks: an animation (the `VP8X` chunk's flag, [`WebpAnimation`]), or a still image, lossless
/// (`VP8L`) or lossy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WebpKind {
    Animated,
    Lossless,
    Lossy,
}

/// The kind of a WebP, from one walk of its chunks.
pub(crate) fn webp_kind(bytes: &[u8]) -> WebpKind {
    let mut animation = WebpAnimation::default();
    let mut lossless = false;
    for c in webp_chunks(bytes) {
        animation.see(&c);
        lossless |= webp_lossless(&c);
    }
    if animation.animated() {
        WebpKind::Animated
    } else if lossless {
        WebpKind::Lossless
    } else {
        WebpKind::Lossy
    }
}
