//! WebP, decoded and written again as lossless VP8L, its other chunks kept.

use std::io::Cursor;

use anyhow::{Context, Result};
use pg_container::webp_chunks;
use pg_model::num;

use super::chunk::chunk_bytes;
use super::op::Op;
use super::raster::{decoded_size, zeroed};

/// A still WebP, written lossless (VP8L, alpha included): a lossy one becomes lossless, larger but with no
/// generation loss. The caller checks for `ANIM`.
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
    let vp8l = webp_chunks(&encoded).find(|c| &c.kind == b"VP8L").context("encoder wrote no VP8L chunk")?;
    let vp8l = chunk_bytes(&encoded, &vp8l)?.to_vec();

    let mut body = b"WEBP".to_vec();
    let mut image_written = false;
    for c in webp_chunks(bytes) {
        // VP8X: flags (4 bytes), then the canvas width and height less one (3 bytes each).
        match (&c.kind, c.data.split_first_chunk::<10>()) {
            (b"VP8X", Some((fields, extra))) => {
                let mut fields = *fields;
                fields[4..7].copy_from_slice(&(nw - 1).to_le_bytes()[..3]);
                fields[7..10].copy_from_slice(&(nh - 1).to_le_bytes()[..3]);
                body.extend_from_slice(chunk_bytes(bytes, &c)?.get(..8).context("damaged WebP")?);
                body.extend_from_slice(&fields);
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
    let mut out = b"RIFF".to_vec();
    out.extend_from_slice(&u32::try_from(body.len())?.to_le_bytes());
    out.extend(body);
    Ok(out)
}
