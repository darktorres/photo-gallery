//! Decoded rasters: their size held to the decoders' cap, allocated fallibly, and their rows packed, unpacked and
//! turned or cropped.

use std::borrow::Cow;

use crate::{Error, Result};

use super::buffers::reserved;
use super::op::Op;

/// A zeroed buffer of `len` bytes (a size [`crate::codecs::limits::decoded_size`] passed), allocated fallibly.
pub(super) fn zeroed(len: usize) -> Result<Vec<u8>> {
    let mut buf = reserved(len)?;
    buf.resize(len, 0);
    Ok(buf)
}

/// Rows of `bits`-per-pixel samples (MSB first, as PNG and BMP pack them) to one byte per pixel.
pub(super) fn unpack(rows: &[u8], stride: usize, w: usize, h: usize, bits: usize) -> Result<Vec<u8>> {
    let per_byte = 8 / bits;
    let mask = u8::try_from((1u16 << bits) - 1).map_err(|e| Error::invalid(e.to_string()))?;
    let mut out = reserved(w * h)?;
    // (`max`: only a raster no pixel wide has a zero stride, and it reads nothing.)
    for row in rows.chunks(stride.max(1)).take(h) {
        let values = row.iter().flat_map(|&byte| (1..=per_byte).map(move |i| (byte >> (8 - bits * i)) & mask));
        out.extend(values.take(w));
    }
    if out.len() != w * h {
        return Err(Error::invalid("the image data is short"));
    }
    Ok(out)
}

/// One byte per pixel to rows `stride` bytes apart of `bits`-per-pixel samples, MSB first: the inverse of
/// [`unpack`]. `values` holds `w` × `h` of them.
pub(crate) fn pack(values: &[u8], stride: usize, w: usize, h: usize, bits: usize) -> Result<Vec<u8>> {
    let per_byte = 8 / bits;
    let mut out = zeroed(stride * h)?;
    // (`max`: a zero stride or width only comes with no pixels.)
    for (row, line) in out.chunks_exact_mut(stride.max(1)).zip(values.chunks_exact(w.max(1))) {
        for (byte, group) in row.iter_mut().zip(line.chunks(per_byte)) {
            for (i, &value) in (1..).zip(group) {
                *byte |= value << (8 - bits * i);
            }
        }
    }
    Ok(out)
}

/// `op` on a raster of rows `stride` bytes apart, `bits` per pixel; the result's rows are `new_stride(w')` bytes
/// apart. Returns it with the new width and height.
pub(super) fn apply_rows(
    rows: &[u8],
    stride: usize,
    w: usize,
    h: usize,
    bits: usize,
    op: Op,
    new_stride: impl Fn(usize) -> usize,
) -> Result<(Vec<u8>, usize, usize)> {
    if bits < 8 {
        let (values, nw, nh) = op.apply(&unpack(rows, stride, w, h, bits)?, w, h, 1)?;
        let s = new_stride(nw);
        return Ok((pack(&values, s, nw, nh, bits)?, nw, nh));
    }
    let unit = bits / 8;
    let row_len = w * unit;
    // The pixels packed, one row after the other: the rows themselves when they have no padding (a copy is made only
    // when they do).
    let packed: Cow<'_, [u8]> = if stride == row_len {
        Cow::Borrowed(rows.get(..w * h * unit).ok_or_else(|| Error::invalid("the image data is short"))?)
    } else {
        // (`max`: only a raster no pixel wide has a zero stride, and it reads nothing.)
        let mut packed = reserved(w * h * unit)?;
        for row in rows.chunks(stride.max(1)).take(h) {
            let kept = row.get(..row_len).ok_or_else(|| Error::invalid("the image data is short"))?;
            packed.extend_from_slice(kept);
        }
        Cow::Owned(packed)
    };
    if packed.len() != w * h * unit {
        return Err(Error::invalid("the image data is short"));
    }
    let (pixels, nw, nh) = op.apply(&packed, w, h, unit)?;
    let s = new_stride(nw);
    // The result's rows are unpadded: it is the result as it is.
    if s == nw * unit {
        return Ok((pixels, nw, nh));
    }
    let mut out = zeroed(s * nh)?;
    for (line, row) in out.chunks_mut(s.max(1)).zip(pixels.chunks_exact((nw * unit).max(1))) {
        let dst = line.get_mut(..row.len()).ok_or_else(|| Error::invalid("a row wider than its stride"))?;
        dst.copy_from_slice(row);
    }
    Ok((out, nw, nh))
}
