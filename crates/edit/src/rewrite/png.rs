//! PNG, decoded and encoded again losslessly, its other chunks kept.

use std::io::Cursor;

use anyhow::{Context, Result, bail};
use pg_container::png_chunks;
use pg_model::num;

use super::chunk::{chunk_bytes, png_chunk};
use super::op::Op;
use super::raster::{apply_rows, decoded_size, zeroed};

/// A still PNG (an animation's other frames would be lost: the caller checks for `acTL`).
pub(crate) fn rewrite(bytes: &[u8], op: Op) -> Result<Vec<u8>> {
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::IDENTITY);
    let mut reader = decoder.read_info()?;
    let info = reader.info().clone();
    let (w, h) = (num::index(info.width), num::index(info.height));
    let bits = info.bits_per_pixel();
    // The stored rows, and the pixels at a byte or more each that the turn or crop works on (samples of fewer bits
    // are unpacked to a byte): both held to the cap before either is allocated. (The png crate's own limit counts
    // one row, and `output_buffer_size` only stops at `isize::MAX`.)
    let stored = reader.output_buffer_size().context("the PNG is too large to decode")?;
    decoded_size("the PNG", &[stored])?;
    decoded_size("the PNG", &[w, h, bits.div_ceil(8)])?;
    let mut raw = zeroed(stored)?;
    let frame = reader.next_frame(&mut raw)?;
    let (pixels, nw, nh) = apply_rows(&raw, frame.line_size, w, h, bits, op, |nw| (nw * bits).div_ceil(8))?;
    let (nw, nh) = (u32::try_from(nw)?, u32::try_from(nh)?);

    // Encode the new pixels in the same color type and depth, and keep only the image data from it.
    let mut encoded = Vec::new();
    let mut encoder = png::Encoder::new(&mut encoded, nw, nh);
    encoder.set_color(info.color_type);
    encoder.set_depth(info.bit_depth);
    if let Some(palette) = &info.palette {
        encoder.set_palette(palette.to_vec());
    }
    if let Some(trns) = &info.trns {
        encoder.set_trns(trns.to_vec());
    }
    encoder.set_compression(png::Compression::High);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(&pixels)?;
    writer.finish()?;
    let mut idat = Vec::new();
    for c in png_chunks(&encoded).filter(|c| &c.kind == b"IDAT") {
        idat.extend_from_slice(chunk_bytes(&encoded, &c)?);
    }

    let mut out = bytes.get(..8).context("damaged PNG")?.to_vec();
    let mut idat_written = false;
    for c in png_chunks(bytes) {
        let raw_chunk = chunk_bytes(bytes, &c)?;
        match &c.kind {
            b"IHDR" => {
                // Width and height (4 bytes each); bit depth, color type, compression and filter method, kept; the
                // interlace method.
                let (Some(kept), Some(extra)) = (c.data.get(8..12), c.data.get(13..)) else {
                    bail!("damaged PNG header")
                };
                let mut ihdr = Vec::with_capacity(c.data.len());
                ihdr.extend_from_slice(&nw.to_be_bytes());
                ihdr.extend_from_slice(&nh.to_be_bytes());
                ihdr.extend_from_slice(kept);
                ihdr.push(0); // written non-interlaced
                ihdr.extend_from_slice(extra);
                out.extend(png_chunk(b"IHDR", &ihdr)?);
            }
            b"IDAT" if !idat_written => {
                out.extend_from_slice(&idat);
                idat_written = true;
            }
            b"IDAT" => {}
            b"pHYs" if op.quarter() => match <[u8; 9]>::try_from(c.data) {
                Ok(mut phys) => {
                    phys[..8].rotate_left(4); // pixels per unit, x and y
                    out.extend(png_chunk(b"pHYs", &phys)?);
                }
                Err(_) => out.extend_from_slice(raw_chunk),
            },
            _ => out.extend_from_slice(raw_chunk),
        }
    }
    Ok(out)
}
