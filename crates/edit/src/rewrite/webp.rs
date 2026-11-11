//! WebP, decoded and written again as lossless VP8L, its other chunks kept.

use std::io::Cursor;

use anyhow::{Context, Result};
use pg_container::{riff, vp8x_with_canvas, webp_chunks, webp_lossless};
use pg_model::num;

use super::chunk::chunk_bytes;
use super::op::Op;
use super::raster::{decoded_size, zeroed};

/// A still WebP, written lossless (VP8L, alpha included): a lossy one becomes lossless, larger but with no
/// generation loss. The caller checks that it is no animation.
pub(crate) fn rewrite(bytes: &[u8], op: Op) -> Result<Vec<u8>> {
    let mut decoder = image_webp::WebPDecoder::new(Cursor::new(bytes))?;
    let (w, h) = decoder.dimensions();
    let alpha = decoder.has_alpha();
    let size = decoder.output_buffer_size().context("the WebP is too large to decode")?;
    let mut rgba = zeroed(decoded_size("the WebP", &[size])?)?;
    decoder.read_image(&mut rgba)?;
    let unit = if alpha { 4 } else { 3 };
    let (pixels, nw, nh) = op.apply(&rgba, num::index(w), num::index(h), unit)?;
    let (nw, nh) = (u32::try_from(nw)?, u32::try_from(nh)?);
    let mut encoded = Vec::new();
    let color = if alpha { image_webp::ColorType::Rgba8 } else { image_webp::ColorType::Rgb8 };
    image_webp::WebPEncoder::new(&mut encoded).encode(&pixels, nw, nh, color)?;
    let vp8l = webp_chunks(&encoded).find(webp_lossless).context("encoder wrote no VP8L chunk")?;
    let vp8l = chunk_bytes(&encoded, &vp8l)?.to_vec();

    let mut body = b"WEBP".to_vec();
    let mut image_written = false;
    for c in webp_chunks(bytes) {
        // VP8X: its flags and canvas, the canvas set to the new size.
        match (&c.kind, c.data.split_first_chunk::<10>()) {
            (b"VP8X", Some((fields, extra))) => {
                body.extend_from_slice(chunk_bytes(bytes, &c)?.get(..8).context("damaged WebP")?);
                body.extend_from_slice(&vp8x_with_canvas(*fields, nw, nh));
                body.extend_from_slice(extra);
                if c.data.len() % 2 == 1 {
                    body.push(0);
                }
            }
            // The image (a lossy one's alpha is in its ALPH chunk; VP8L carries its own).
            (b"VP8L" | b"VP8 " | b"ALPH", _) => {
                if !std::mem::replace(&mut image_written, true) {
                    body.extend_from_slice(&vp8l);
                }
            }
            _ => body.extend_from_slice(chunk_bytes(bytes, &c)?),
        }
    }
    riff(&body).context("the WebP is too large for a RIFF file")
}
