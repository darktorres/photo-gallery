//! Decoded rasters: their size held to the decoders' cap, allocated fallibly, and their rows packed, unpacked and
//! turned or cropped.

use anyhow::{Context, Result, bail, ensure};

use super::op::Op;

/// The bytes of a decoded raster, the product of `factors` (width, height, bytes per pixel), held to the one cap
/// for decoded buffers ([`pg_codec::max_decode_bytes`], as the app's decoders hold theirs). A header that declares
/// a huge image (a damaged one) is an error here, before anything that size is allocated: an allocation failure
/// would abort the process, with no error recorded.
pub(super) fn decoded_size(what: &str, factors: &[usize]) -> Result<usize> {
    let len = factors.iter().try_fold(1usize, |n, &f| n.checked_mul(f));
    match len {
        Some(len) if u64::try_from(len).is_ok_and(|len| len <= pg_codec::max_decode_bytes()) => Ok(len),
        _ => bail!("{what} is too large to decode"),
    }
}

/// A zeroed buffer of `len` bytes (a size [`decoded_size`] passed), allocated fallibly: too little memory is an
/// error, not an abort.
pub(super) fn zeroed(len: usize) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    buf.try_reserve_exact(len).context("not enough memory to decode the image")?;
    buf.resize(len, 0);
    Ok(buf)
}

/// Rows of `bits`-per-pixel samples (MSB first, as PNG and BMP pack them) to one byte per pixel.
pub(super) fn unpack(rows: &[u8], stride: usize, w: usize, h: usize, bits: usize) -> Result<Vec<u8>> {
    let per_byte = 8 / bits;
    let mask = u8::try_from((1u16 << bits) - 1)?;
    let mut out = Vec::with_capacity(w * h);
    // (`max`: only a raster no pixel wide has a zero stride, and it reads nothing.)
    for row in rows.chunks(stride.max(1)).take(h) {
        let values = row.iter().flat_map(|&byte| (1..=per_byte).map(move |i| (byte >> (8 - bits * i)) & mask));
        out.extend(values.take(w));
    }
    ensure!(out.len() == w * h, "the image data is short");
    Ok(out)
}

/// One byte per pixel to rows `stride` bytes apart of `bits`-per-pixel samples, MSB first: the inverse of
/// [`unpack`]. `values` holds `w` × `h` of them.
pub(crate) fn pack(values: &[u8], stride: usize, w: usize, h: usize, bits: usize) -> Vec<u8> {
    let per_byte = 8 / bits;
    let mut out = vec![0u8; stride * h];
    // (`max`: a zero stride or width only comes with no pixels.)
    for (row, line) in out.chunks_exact_mut(stride.max(1)).zip(values.chunks_exact(w.max(1))) {
        for (byte, group) in row.iter_mut().zip(line.chunks(per_byte)) {
            for (i, &value) in (1..).zip(group) {
                *byte |= value << (8 - bits * i);
            }
        }
    }
    out
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
        return Ok((pack(&values, s, nw, nh, bits), nw, nh));
    }
    let unit = bits / 8;
    // (`max`: only a raster no pixel wide has a zero stride, and it reads nothing.)
    let mut packed = Vec::with_capacity(w * h * unit);
    for row in rows.chunks(stride.max(1)).take(h) {
        packed.extend_from_slice(row.get(..w * unit).context("the image data is short")?);
    }
    ensure!(packed.len() == w * h * unit, "the image data is short");
    let (pixels, nw, nh) = op.apply(&packed, w, h, unit)?;
    let s = new_stride(nw);
    let mut out = vec![0u8; s * nh];
    for (line, row) in out.chunks_mut(s.max(1)).zip(pixels.chunks_exact((nw * unit).max(1))) {
        line.get_mut(..row.len()).context("a row wider than its stride")?.copy_from_slice(row);
    }
    Ok((out, nw, nh))
}
