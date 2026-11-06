//! A file's pixels replaced in its own format, with everything else in it kept: the turns of `rotate` and the
//! crops of `crop`. Only the pixel data and the fields that describe it (size, placement, pixel density) change;
//! every other chunk or block (ICC profile, EXIF, XMP, text, comments) is copied as it was. JPEG is transformed in
//! the DCT domain; PNG, BMP, still GIF and WebP are decoded and re-encoded losslessly (WebP as lossless VP8L).

use std::io::Cursor;

use anyhow::{Context, Result, bail, ensure};
use turbojpeg::{Transform, TransformCrop, TransformOp, Transformer};

use pg_codec::decode::transform;
use pg_container::{Chunk, GifBlock, gif_blocks, gif_header_len, png_chunks, webp_chunks};
use pg_model::{num, orientation::Orientation};

/// What happens to the stored pixels.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Op {
    /// Turned under an EXIF orientation (1–8).
    Orient(Orientation),
    /// Cut to this rectangle: left, top, width, height.
    Crop([usize; 4]),
}

impl Op {
    /// Width and height swap.
    fn quarter(self) -> bool {
        matches!(self, Op::Orient(o) if o.swaps_axes())
    }

    /// The new size of a `w` × `h` raster.
    fn size(self, w: usize, h: usize) -> (usize, usize) {
        match self {
            Op::Orient(_) if self.quarter() => (h, w),
            Op::Orient(_) => (w, h),
            Op::Crop([_, _, cw, ch]) => (cw, ch),
        }
    }

    /// The pixels of a `w` × `h` raster of `unit` bytes per pixel, with the new width and height. A crop that
    /// reaches outside the raster's rows is an error.
    fn apply(self, pixels: &[u8], w: usize, h: usize, unit: usize) -> Result<(Vec<u8>, usize, usize)> {
        match self {
            Op::Orient(o) => Ok(transform(pixels, w, h, unit, o)),
            Op::Crop([x, y, cw, ch]) => {
                let mut out = Vec::with_capacity(cw * ch * unit);
                // (`max`: a raster no pixel wide has no rows.)
                for row in pixels.chunks_exact((w * unit).max(1)).skip(y).take(ch) {
                    out.extend_from_slice(row.get(x * unit..(x + cw) * unit).context("the crop is outside the image")?);
                }
                ensure!(out.len() == cw * ch * unit, "the crop is outside the image");
                Ok((out, cw, ch))
            }
        }
    }

    /// Where the `w` × `h` frame at (`left`, `top`) of a `sw` × `sh` screen (a GIF's logical screen) goes: its
    /// new place, the part of it kept (left, top, width, height, in the frame), and the new screen size. `None`
    /// when nothing of it is kept.
    #[expect(clippy::type_complexity, reason = "the placement and the kept part are returned together")]
    fn frame(
        self,
        [left, top, w, h]: [usize; 4],
        sw: usize,
        sh: usize,
    ) -> Option<((usize, usize), [usize; 4], (usize, usize))> {
        match self {
            Op::Orient(o) => {
                let (a, b) = (o.shown_pixel(left, top, sw, sh), o.shown_pixel(left + w - 1, top + h - 1, sw, sh));
                Some(((a.0.min(b.0), a.1.min(b.1)), [0, 0, w, h], self.size(sw, sh)))
            }
            Op::Crop([cx, cy, cw, ch]) => {
                let (x0, y0) = (left.max(cx), top.max(cy));
                let (x1, y1) = ((left + w).min(cx + cw), (top + h).min(cy + ch));
                (x0 < x1 && y0 < y1).then_some(((x0 - cx, y0 - cy), [x0 - left, y0 - top, x1 - x0, y1 - y0], (cw, ch)))
            }
        }
    }
}

// --- JPEG -------------------------------------------------------------------------------------------------------

/// A JPEG transform that can't be done perfectly, so the file is left as it is and the orientation tag carries the
/// turn (`rotate`). Every other error is a failure.
#[derive(Debug)]
pub struct Imperfect(pub String);

impl std::fmt::Display for Imperfect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "not a perfect transform: {}", self.0)
    }
}

impl std::error::Error for Imperfect {}

/// In the DCT domain (libjpeg-turbo's lossless transform), every marker copied. Turns are perfect transforms only:
/// one that would have to drop or smear partial edge blocks is [`Imperfect`]. A crop's left and top must be on the
/// MCU grid ([`jpeg_mcu`]).
pub(crate) fn jpeg(bytes: &[u8], op: Op) -> Result<Vec<u8>> {
    let mut t = Transform::default();
    match op {
        Op::Orient(o) => {
            t.op = match o.get() {
                2 => TransformOp::Hflip,
                3 => TransformOp::Rot180,
                4 => TransformOp::Vflip,
                5 => TransformOp::Transpose,
                6 => TransformOp::Rot90,
                7 => TransformOp::Transverse,
                8 => TransformOp::Rot270,
                _ => return Ok(bytes.to_vec()),
            };
            t.perfect = true;
        }
        Op::Crop([x, y, w, h]) => t.crop = Some(TransformCrop { x, y, width: Some(w), height: Some(h) }),
    }
    // Keep the entropy coding kind: progressive stays progressive; baseline gets optimized Huffman tables (the
    // transform can't reuse the original ones).
    // A JPEG whose structure is damaged before its scan is refused: the transform would re-encode what is left.
    let Some(progressive) = pg_container::jpeg_is_progressive(bytes) else {
        return Err(Imperfect("the JPEG's structure is damaged before its scan".into()).into());
    };
    t.progressive = progressive;
    t.optimize = !progressive;
    let perfect = t.perfect;
    match Transformer::new()?.transform_to_vec(&t, bytes) {
        Ok(out) => Ok(out),
        // A perfect transform fails when it would have to cut through a block: that is imperfect, not a failure.
        Err(e) if perfect => Err(Imperfect(e.to_string()).into()),
        Err(e) => Err(e.into()),
    }
}

/// The JPEG's minimum coded unit, width and height: the grid a lossless crop starts on.
pub(crate) fn jpeg_mcu(bytes: &[u8]) -> Result<(usize, usize)> {
    let header = turbojpeg::read_header(bytes)?;
    Ok((header.subsamp.mcu_width(), header.subsamp.mcu_height()))
}

// --- Packed rasters ---------------------------------------------------------------------------------------------

/// Rows of `bits`-per-pixel samples (MSB first, as PNG and BMP pack them) to one byte per pixel.
fn unpack(rows: &[u8], stride: usize, w: usize, h: usize, bits: usize) -> Result<Vec<u8>> {
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
fn apply_rows(
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

// --- PNG --------------------------------------------------------------------------------------------------------

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
fn chunk_bytes<'a>(bytes: &'a [u8], c: &Chunk<'_>) -> Result<&'a [u8]> {
    bytes.get(c.at..c.at + c.len).context("a chunk outside its file")
}

/// A still PNG (an animation's other frames would be lost: the caller checks for `acTL`).
pub(crate) fn png(bytes: &[u8], op: Op) -> Result<Vec<u8>> {
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::IDENTITY);
    let mut reader = decoder.read_info()?;
    let info = reader.info().clone();
    let mut raw = vec![0u8; reader.output_buffer_size().context("PNG too large")?];
    let frame = reader.next_frame(&mut raw)?;
    let (w, h) = (num::index(info.width), num::index(info.height));
    let bits = info.bits_per_pixel();
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

// --- WebP -------------------------------------------------------------------------------------------------------

/// A still WebP, written lossless (VP8L, alpha included): a lossy one becomes lossless, larger but with no
/// generation loss. The caller checks for `ANIM`.
pub(crate) fn webp(bytes: &[u8], op: Op) -> Result<Vec<u8>> {
    let mut decoder = image_webp::WebPDecoder::new(Cursor::new(bytes))?;
    let (w, h) = decoder.dimensions();
    let alpha = decoder.has_alpha();
    let mut rgba = vec![0u8; decoder.output_buffer_size().context("WebP too large")?];
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

// --- GIF --------------------------------------------------------------------------------------------------------

fn sub_blocks(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + data.len() / 255 + 1);
    for chunk in data.chunks(255) {
        out.push(u8::try_from(chunk.len()).unwrap_or(u8::MAX));
        out.extend_from_slice(chunk);
    }
    out.push(0);
    out
}

/// A still GIF; `None` for an animation (more than one image).
pub(crate) fn gif(bytes: &[u8], op: Op) -> Result<Option<Vec<u8>>> {
    let mut blocks = gif_blocks(bytes);
    let (mut images, mut image) = (0, None);
    for b in blocks.by_ref() {
        if let GifBlock::Image { range } = b {
            images += 1;
            image = Some(range);
        }
    }
    if !blocks.complete() {
        bail!("damaged GIF");
    }
    let (1, Some(range)) = (images, image) else { return Ok(None) };
    let header = gif_header_len(bytes).context("damaged GIF")?;
    let byte_at = |i: usize| bytes.get(i).copied().context("damaged GIF");
    let u16_at = |i: usize| {
        bytes
            .get(i..)
            .and_then(<[u8]>::first_chunk::<2>)
            .map(|&b| usize::from(u16::from_le_bytes(b)))
            .context("damaged GIF")
    };
    let (sw, sh) = (u16_at(6)?, u16_at(8)?);
    let d = range.start;
    let (left, top, w, h, packed) = (u16_at(d + 1)?, u16_at(d + 3)?, u16_at(d + 5)?, u16_at(d + 7)?, byte_at(d + 9)?);
    let table = if packed & 0x80 != 0 { 3 << ((packed & 7) + 1) } else { 0 };
    let code_at = d + 10 + table;
    let min_code = byte_at(code_at)?;

    // The LZW stream, from its sub-blocks.
    let mut lzw = Vec::new();
    let mut rest = bytes.get(code_at + 1..).context("damaged GIF")?;
    loop {
        let Some((&n, after)) = rest.split_first() else { bail!("damaged GIF") };
        if n == 0 {
            break;
        }
        let Some((block, after)) = after.split_at_checked(usize::from(n)) else { bail!("damaged GIF") };
        lzw.extend_from_slice(block);
        rest = after;
    }
    let mut indices = weezl::decode::Decoder::new(weezl::BitOrder::Lsb, min_code).decode(&lzw)?;
    if indices.len() < w * h {
        bail!("GIF image data is short");
    }
    indices.truncate(w * h);
    // (An image no pixel wide has no rows to reorder.)
    if packed & 0x40 != 0 && w > 0 {
        // Interlaced: rows stored in four passes. Each stored row is paired with its place, then put back in order.
        let order = (0..h).step_by(8).chain((4..h).step_by(8)).chain((2..h).step_by(4)).chain((1..h).step_by(2));
        let mut rows: Vec<(usize, &[u8])> = order.zip(indices.chunks_exact(w)).collect();
        rows.sort_unstable_by_key(|&(y, _)| y);
        indices = rows.into_iter().flat_map(|(_, row)| row).copied().collect();
    }
    // The frame's place on the logical screen changes with it.
    let ((nleft, ntop), kept, (nsw, nsh)) =
        op.frame([left, top, w, h], sw, sh).context("nothing of the GIF's image is left")?;
    let (kept_pixels, kw, kh) = Op::Crop(kept).apply(&indices, w, h, 1)?;
    let (pixels, nw, nh) = match op {
        Op::Orient(_) => op.apply(&kept_pixels, kw, kh, 1)?,
        Op::Crop(_) => (kept_pixels, kw, kh),
    };
    let data = weezl::encode::Encoder::new(weezl::BitOrder::Lsb, min_code).encode(&pixels)?;

    let mut out = bytes.get(..header).context("damaged GIF")?.to_vec();
    let screen = out.first_chunk_mut::<10>().context("damaged GIF")?;
    screen[6..8].copy_from_slice(&u16::try_from(nsw)?.to_le_bytes());
    screen[8..10].copy_from_slice(&u16::try_from(nsh)?.to_le_bytes());
    out.extend_from_slice(bytes.get(header..d).context("damaged GIF")?);
    out.push(0x2C);
    for v in [nleft, ntop, nw, nh] {
        out.extend_from_slice(&u16::try_from(v)?.to_le_bytes());
    }
    out.push(packed & !0x40); // written non-interlaced
    out.extend_from_slice(bytes.get(d + 10..code_at + 1).context("damaged GIF")?); // local color table, LZW code size
    out.extend(sub_blocks(&data));
    out.extend_from_slice(bytes.get(range.end..).context("damaged GIF")?);
    Ok(Some(out))
}

// --- BMP --------------------------------------------------------------------------------------------------------

/// An uncompressed BMP; `None` for what this doesn't rewrite (RLE compression, OS/2 headers, data after the
/// pixels).
pub(crate) fn bmp(bytes: &[u8], op: Op) -> Result<Option<Vec<u8>>> {
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
