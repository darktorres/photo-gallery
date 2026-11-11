//! A file's EXIF orientation, read alone ([`orientation`]) and set in place ([`patch_orientation`]). Read and write
//! find the same EXIF block, so they are together.

use std::ops::Range;

use pg_container::{chunk_data, fix_png_crc, jpeg_exif, jpeg_segments, png_chunks, webp_chunks, webp_exif};
use pg_model::format::Format;
use pg_model::orientation::Orientation;

use crate::exif::Exif;
use crate::tiff::{TIFF_BYTE, TIFF_LONG, TIFF_LONG8, TIFF_SHORT, Tiff};

/// The EXIF orientation (1–8) of a file, alone; 1 when absent or unreadable.
pub fn orientation(bytes: &[u8], format: Format) -> Orientation {
    exif_block(bytes, format)
        .and_then(|block| Exif::read(bytes.get(block)?, format == Format::Tiff))
        .unwrap_or_default()
        .orientation()
}

/// Where the EXIF block (a TIFF structure) of a file is, if it has one.
fn exif_block(bytes: &[u8], format: Format) -> Option<Range<usize>> {
    match format {
        Format::Jpeg => jpeg_segments(bytes).find_map(|s| jpeg_exif(&s)),
        Format::Png => png_chunks(bytes).find(|c| &c.kind == b"eXIf").map(|c| chunk_data(&c)),
        Format::Webp => webp_chunks(bytes).find(|c| &c.kind == b"EXIF").map(|c| webp_exif(&c)),
        Format::Tiff => Some(0..bytes.len()),
        Format::Gif | Format::Bmp => None,
    }
}

/// The value bytes to write for each orientation entry in a TIFF block's IFD0 (those [`Tiff::orientation_entries`]
/// yields): the range of the entry's first value in the block, and `o` at that entry's width (BYTE, SHORT, LONG or
/// LONG8), in the block's byte order.
fn tiff_orientation_patches(block: &[u8], bigtiff: bool, o: Orientation) -> Option<Vec<(Range<usize>, Vec<u8>)>> {
    let tiff = Tiff::new(block, bigtiff)?;
    let le = tiff.little_endian;
    let v = o.get();
    let patches = tiff
        .orientation_entries()
        .filter_map(|(range, e)| {
            let value: Vec<u8> = match e.kind {
                TIFF_BYTE => vec![v],
                TIFF_SHORT => (if le { u16::from(v).to_le_bytes() } else { u16::from(v).to_be_bytes() }).to_vec(),
                TIFF_LONG => (if le { u32::from(v).to_le_bytes() } else { u32::from(v).to_be_bytes() }).to_vec(),
                TIFF_LONG8 => (if le { u64::from(v).to_le_bytes() } else { u64::from(v).to_be_bytes() }).to_vec(),
                _ => return None,
            };
            let first = range.start..range.start.checked_add(value.len())?;
            // The entry's value holds at least one value of its width (its count is not 0).
            (first.end <= range.end).then_some((first, value))
        })
        .collect();
    Some(patches)
}

/// The file with its EXIF orientation set to `o`, changed in place (same size, nothing moves), or `None` when
/// the file has no orientation tag to change (adding one is a job for ExifTool). Every IFD0 orientation entry the
/// reader could take is set, each at its own width, so the reader sees `o` whichever entry it takes.
pub fn patch_orientation(bytes: &[u8], format: Format, o: Orientation) -> Option<Vec<u8>> {
    let block = exif_block(bytes, format)?;
    let patches = tiff_orientation_patches(bytes.get(block.clone())?, format == Format::Tiff, o)?;
    if patches.is_empty() {
        return None;
    }
    let mut out = bytes.to_vec();
    for (range, value) in patches {
        let at = block.start.checked_add(range.start)?..block.start.checked_add(range.end)?;
        out.get_mut(at)?.copy_from_slice(&value);
    }
    if format == Format::Png {
        // The eXIf chunk's CRC covers its type and data.
        let at = png_chunks(&out).find(|c| &c.kind == b"eXIf")?.at;
        fix_png_crc(&mut out, at)?;
    }
    Some(out)
}
