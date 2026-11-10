//! An uncompressed BMP, its rows turned or cropped and its header fields set to the new size.

use anyhow::{Context, Result, bail};
use pg_model::num;

use super::op::Op;
use super::raster::apply_rows;

/// An uncompressed BMP; `None` for what this doesn't rewrite (RLE compression, OS/2 headers, data after the
/// pixels).
pub(crate) fn rewrite(bytes: &[u8], op: Op) -> Result<Option<Vec<u8>>> {
    let u16_at = |i: usize| bytes.get(i..).and_then(<[u8]>::first_chunk::<2>).map(|&b| u16::from_le_bytes(b));
    let u32_at = |i: usize| bytes.get(i..).and_then(<[u8]>::first_chunk::<4>).map(|&b| u32::from_le_bytes(b));
    let (Some(offset), Some(dib)) = (u32_at(10), u32_at(14)) else { bail!("damaged BMP") };
    if dib < 40 {
        return Ok(None);
    }
    let (Some(w), Some(h), Some(bits), Some(compression)) = (u32_at(18), u32_at(22), u16_at(28), u32_at(30)) else {
        bail!("damaged BMP")
    };
    let (w, h, bits) = (w.cast_signed(), h.cast_signed(), usize::from(bits));
    // BI_RGB, BI_BITFIELDS, BI_ALPHABITFIELDS: plain rows. RLE and embedded JPEG/PNG: not rewritten.
    if !matches!(compression, 0 | 3 | 6) || ![1, 2, 4, 8, 16, 24, 32].contains(&bits) || w <= 0 || h == 0 {
        return Ok(None);
    }
    let (w, top_down, h) = (usize::try_from(w)?, h < 0, num::index(h.unsigned_abs()));
    let stride = (w * bits).div_ceil(32) * 4;
    let offset = num::index(offset);
    let Some((head, stored)) = bytes.split_at_checked(offset).filter(|(_, stored)| stored.len() == stride * h) else {
        return Ok(None);
    };
    // Rows top to bottom (`stride` is at least 4: `w` is positive).
    let rows: Vec<u8> =
        if top_down { stored.to_vec() } else { stored.chunks_exact(stride).rev().flatten().copied().collect() };
    let new_stride = |nw: usize| (nw * bits).div_ceil(32) * 4;
    let (pixels, nw, nh) = apply_rows(&rows, stride, w, h, bits, op, new_stride)?;
    let ns = new_stride(nw);

    let mut out = head.to_vec();
    let Some(fields) = out.first_chunk_mut::<38>() else { bail!("damaged BMP") };
    // The file's size: the headers, then `nh` rows of `ns` bytes (all of `pixels`).
    fields[2..6].copy_from_slice(&u32::try_from(offset + pixels.len())?.to_le_bytes());
    fields[18..22].copy_from_slice(&i32::try_from(nw)?.to_le_bytes());
    let nh_field = i32::try_from(nh)?;
    fields[22..26].copy_from_slice(&(if top_down { -nh_field } else { nh_field }).to_le_bytes());
    fields[34..38].copy_from_slice(&u32::try_from(ns * nh)?.to_le_bytes());
    if op.quarter() {
        // Pixels per metre, x and y.
        out.get_mut(38..46).context("damaged BMP")?.rotate_left(4);
    }
    if top_down {
        out.extend_from_slice(&pixels);
    } else {
        // A row holds at least one pixel, so `ns` is positive; `max` only keeps `chunks_exact` from panicking.
        for row in pixels.chunks_exact(ns.max(1)).rev() {
            out.extend_from_slice(row);
        }
    }
    Ok(Some(out))
}
