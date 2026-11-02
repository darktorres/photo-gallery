//! A file's pixels replaced in its own format, with everything else in it kept: the turns of `rotate` and the
//! crops of `crop`. Only the pixel data and the fields that describe it (size, placement, pixel density) change;
//! every other chunk or block (ICC profile, EXIF, XMP, text, comments) is copied as it was. JPEG is transformed in
//! the DCT domain; PNG, BMP, still GIF and WebP are decoded and re-encoded losslessly (WebP as lossless VP8L).

use std::io::Cursor;

use anyhow::{Context, Result, bail};
use turbojpeg::{Transform, TransformCrop, TransformOp, Transformer};

use crate::container;
use crate::container::{GifBlock, gif_blocks, gif_header_len, png_chunks, webp_chunks};
use crate::decode::transform;
use crate::orientation;

/// What happens to the stored pixels.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Op {
    /// Turned under an EXIF orientation (1–8).
    Orient(u8),
    /// Cut to this rectangle: left, top, width, height.
    Crop([usize; 4]),
}

impl Op {
    /// Width and height swap.
    fn quarter(self) -> bool {
        matches!(self, Op::Orient(o) if crate::orientation::swaps_axes(o))
    }

    /// The new size of a `w` × `h` raster.
    fn size(self, w: usize, h: usize) -> (usize, usize) {
        match self {
            Op::Orient(_) if self.quarter() => (h, w),
            Op::Orient(_) => (w, h),
            Op::Crop([_, _, cw, ch]) => (cw, ch),
        }
    }

    /// The pixels of a `w` × `h` raster of `unit` bytes per pixel, with the new width and height.
    fn apply(self, pixels: &[u8], w: usize, h: usize, unit: usize) -> (Vec<u8>, usize, usize) {
        match self {
            Op::Orient(o) => transform(pixels, w, h, unit, o),
            Op::Crop([x, y, cw, ch]) => {
                let mut out = Vec::with_capacity(cw * ch * unit);
                for row in y..y + ch {
                    out.extend_from_slice(&pixels[(row * w + x) * unit..(row * w + x + cw) * unit]);
                }
                (out, cw, ch)
            }
        }
    }

    /// Where the `w` × `h` frame at (`left`, `top`) of a `sw` × `sh` screen (a GIF's logical screen) goes: its
    /// new place, the part of it kept (left, top, width, height, in the frame), and the new screen size. `None`
    /// when nothing of it is kept.
    #[allow(clippy::type_complexity)]
    fn frame(
        self,
        [left, top, w, h]: [usize; 4],
        sw: usize,
        sh: usize,
    ) -> Option<((usize, usize), [usize; 4], (usize, usize))> {
        match self {
            Op::Orient(o) => {
                let (a, b) = (
                    orientation::shown_pixel(o, left, top, sw, sh),
                    orientation::shown_pixel(o, left + w - 1, top + h - 1, sw, sh),
                );
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

/// In the DCT domain (libjpeg-turbo's lossless transform), every marker copied. Turns are perfect transforms only:
/// one that would have to drop or smear partial edge blocks is an error. A crop's left and top must be on the
/// MCU grid ([`jpeg_mcu`]).
pub(crate) fn jpeg(bytes: &[u8], op: Op) -> Result<Vec<u8>> {
    let mut t = Transform::default();
    match op {
        Op::Orient(o) => {
            t.op = match o {
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
    let Some(progressive) = container::jpeg_is_progressive(bytes) else {
        bail!("the JPEG's structure is damaged before its scan; it is left as it is");
    };
    t.progressive = progressive;
    t.optimize = !progressive;
    Ok(Transformer::new()?.transform_to_vec(&t, bytes)?)
}

/// The JPEG's minimum coded unit, width and height: the grid a lossless crop starts on.
pub(crate) fn jpeg_mcu(bytes: &[u8]) -> Result<(usize, usize)> {
    let header = turbojpeg::read_header(bytes)?;
    Ok((header.subsamp.mcu_width(), header.subsamp.mcu_height()))
}

// --- Packed rasters ---------------------------------------------------------------------------------------------

/// Rows of `bits`-per-pixel samples (MSB first, as PNG and BMP pack them) to one byte per pixel.
fn unpack(rows: &[u8], stride: usize, w: usize, h: usize, bits: usize) -> Vec<u8> {
    let per_byte = 8 / bits;
    let mask = ((1u16 << bits) - 1) as u8;
    let mut out = Vec::with_capacity(w * h);
    for y in 0..h {
        let row = &rows[y * stride..];
        for x in 0..w {
            let shift = 8 - bits * (x % per_byte + 1);
            out.push((row[x / per_byte] >> shift) & mask);
        }
    }
    out
}

pub(crate) fn pack(values: &[u8], stride: usize, w: usize, h: usize, bits: usize) -> Vec<u8> {
    let per_byte = 8 / bits;
    let mut out = vec![0u8; stride * h];
    for y in 0..h {
        for x in 0..w {
            let shift = 8 - bits * (x % per_byte + 1);
            out[y * stride + x / per_byte] |= values[y * w + x] << shift;
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
) -> (Vec<u8>, usize, usize) {
    if bits < 8 {
        let (values, nw, nh) = op.apply(&unpack(rows, stride, w, h, bits), w, h, 1);
        let s = new_stride(nw);
        return (pack(&values, s, nw, nh, bits), nw, nh);
    }
    let unit = bits / 8;
    let packed: Vec<u8> = (0..h).flat_map(|y| &rows[y * stride..y * stride + w * unit]).copied().collect();
    let (pixels, nw, nh) = op.apply(&packed, w, h, unit);
    let s = new_stride(nw);
    let mut out = vec![0u8; s * nh];
    for y in 0..nh {
        out[y * s..y * s + nw * unit].copy_from_slice(&pixels[y * nw * unit..(y + 1) * nw * unit]);
    }
    (out, nw, nh)
}

// --- PNG --------------------------------------------------------------------------------------------------------

pub(crate) fn png_chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 12);
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    out.extend_from_slice(&crc32fast::hash(&out[4..]).to_be_bytes());
    out
}

/// A still PNG (an animation's other frames would be lost: the caller checks for `acTL`).
pub(crate) fn png(bytes: &[u8], op: Op) -> Result<Vec<u8>> {
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::IDENTITY);
    let mut reader = decoder.read_info()?;
    let info = reader.info().clone();
    let mut raw = vec![0u8; reader.output_buffer_size().context("PNG too large")?];
    let frame = reader.next_frame(&mut raw)?;
    let (w, h) = (info.width as usize, info.height as usize);
    let bits = info.color_type.samples() * info.bit_depth as usize;
    let (pixels, nw, nh) = apply_rows(&raw, frame.line_size, w, h, bits, op, |nw| (nw * bits).div_ceil(8));

    // Encode the new pixels in the same color type and depth, and keep only the image data from it.
    let mut encoded = Vec::new();
    let mut encoder = png::Encoder::new(&mut encoded, nw as u32, nh as u32);
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
    let idat: Vec<u8> = png_chunks(&encoded)
        .filter(|c| &c.kind == b"IDAT")
        .flat_map(|c| encoded[c.at..c.at + c.len].to_vec())
        .collect();

    let mut out = bytes[..8].to_vec();
    let mut idat_written = false;
    for c in png_chunks(bytes) {
        let raw_chunk = &bytes[c.at..c.at + c.len];
        match &c.kind {
            b"IHDR" => {
                let mut ihdr = c.data.to_vec();
                ihdr[0..4].copy_from_slice(&(nw as u32).to_be_bytes());
                ihdr[4..8].copy_from_slice(&(nh as u32).to_be_bytes());
                ihdr[12] = 0; // written non-interlaced
                out.extend(png_chunk(b"IHDR", &ihdr));
            }
            b"IDAT" if !idat_written => {
                out.extend_from_slice(&idat);
                idat_written = true;
            }
            b"IDAT" => {}
            b"pHYs" if op.quarter() && c.data.len() == 9 => {
                let mut phys = c.data.to_vec();
                phys[..8].rotate_left(4); // pixels per unit, x and y
                out.extend(png_chunk(b"pHYs", &phys));
            }
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
    let (pixels, nw, nh) = op.apply(&rgba, w as usize, h as usize, unit);
    let mut encoded = Vec::new();
    let color = if alpha { image_webp::ColorType::Rgba8 } else { image_webp::ColorType::Rgb8 };
    image_webp::WebPEncoder::new(&mut encoded).encode(&pixels, nw as u32, nh as u32, color)?;
    let vp8l = webp_chunks(&encoded).find(|c| &c.kind == b"VP8L").context("encoder wrote no VP8L chunk")?;
    let vp8l = encoded[vp8l.at..vp8l.at + vp8l.len].to_vec();

    let mut body = b"WEBP".to_vec();
    let mut image_written = false;
    for c in webp_chunks(bytes) {
        match &c.kind {
            b"VP8X" if c.data.len() >= 10 => {
                let mut data = c.data.to_vec();
                data[4..7].copy_from_slice(&(nw as u32 - 1).to_le_bytes()[..3]);
                data[7..10].copy_from_slice(&(nh as u32 - 1).to_le_bytes()[..3]);
                body.extend_from_slice(&bytes[c.at..c.at + 8]);
                body.extend_from_slice(&data);
                if data.len() % 2 == 1 {
                    body.push(0);
                }
            }
            // The image (a lossy one's alpha is in its ALPH chunk; VP8L carries its own).
            b"VP8L" | b"VP8 " | b"ALPH" => {
                if !std::mem::replace(&mut image_written, true) {
                    body.extend_from_slice(&vp8l);
                }
            }
            _ => body.extend_from_slice(&bytes[c.at..c.at + c.len]),
        }
    }
    let mut out = b"RIFF".to_vec();
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend(body);
    Ok(out)
}

// --- GIF --------------------------------------------------------------------------------------------------------

fn sub_blocks(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + data.len() / 255 + 1);
    for chunk in data.chunks(255) {
        out.push(chunk.len() as u8);
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
    let u16_at = |i: usize| u16::from_le_bytes([bytes[i], bytes[i + 1]]) as usize;
    let (sw, sh) = (u16_at(6), u16_at(8));
    let d = range.start;
    let (left, top, w, h, packed) = (u16_at(d + 1), u16_at(d + 3), u16_at(d + 5), u16_at(d + 7), bytes[d + 9]);
    let table = if packed & 0x80 != 0 { 3 << ((packed & 7) + 1) } else { 0 };
    let code_at = d + 10 + table;
    let min_code = bytes[code_at];

    // The LZW stream, from its sub-blocks.
    let mut lzw = Vec::new();
    let mut i = code_at + 1;
    while bytes[i] != 0 {
        let n = bytes[i] as usize;
        lzw.extend_from_slice(&bytes[i + 1..i + 1 + n]);
        i += 1 + n;
    }
    let mut indices = weezl::decode::Decoder::new(weezl::BitOrder::Lsb, min_code).decode(&lzw)?;
    if indices.len() < w * h {
        bail!("GIF image data is short");
    }
    indices.truncate(w * h);
    if packed & 0x40 != 0 {
        // Interlaced: rows stored in four passes.
        let order: Vec<usize> =
            (0..h).step_by(8).chain((4..h).step_by(8)).chain((2..h).step_by(4)).chain((1..h).step_by(2)).collect();
        let mut rows = vec![0u8; w * h];
        for (stored, &y) in order.iter().enumerate() {
            rows[y * w..(y + 1) * w].copy_from_slice(&indices[stored * w..(stored + 1) * w]);
        }
        indices = rows;
    }
    // The frame's place on the logical screen changes with it.
    let ((nleft, ntop), kept, (nsw, nsh)) =
        op.frame([left, top, w, h], sw, sh).context("nothing of the GIF's image is left")?;
    let (kept_pixels, kw, kh) = Op::Crop(kept).apply(&indices, w, h, 1);
    let (pixels, nw, nh) = match op {
        Op::Orient(_) => op.apply(&kept_pixels, kw, kh, 1),
        Op::Crop(_) => (kept_pixels, kw, kh),
    };
    let data = weezl::encode::Encoder::new(weezl::BitOrder::Lsb, min_code).encode(&pixels)?;

    let mut out = bytes[..header].to_vec();
    out[6..8].copy_from_slice(&(nsw as u16).to_le_bytes());
    out[8..10].copy_from_slice(&(nsh as u16).to_le_bytes());
    out.extend_from_slice(&bytes[header..d]);
    out.push(0x2C);
    for v in [nleft, ntop, nw, nh] {
        out.extend_from_slice(&(v as u16).to_le_bytes());
    }
    out.push(packed & !0x40); // written non-interlaced
    out.extend_from_slice(&bytes[d + 10..code_at + 1]); // local color table, LZW code size
    out.extend(sub_blocks(&data));
    out.extend_from_slice(&bytes[range.end..]);
    Ok(Some(out))
}

// --- BMP --------------------------------------------------------------------------------------------------------

/// An uncompressed BMP; `None` for what this doesn't rewrite (RLE compression, OS/2 headers, data after the
/// pixels).
pub(crate) fn bmp(bytes: &[u8], op: Op) -> Result<Option<Vec<u8>>> {
    let u16_at = |i: usize| bytes.get(i..i + 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
    let u32_at = |i: usize| bytes.get(i..i + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap()));
    let (Some(offset), Some(dib)) = (u32_at(10), u32_at(14)) else { bail!("damaged BMP") };
    if dib < 40 {
        return Ok(None);
    }
    let (Some(w), Some(h), Some(bits), Some(compression)) = (u32_at(18), u32_at(22), u16_at(28), u32_at(30)) else {
        bail!("damaged BMP")
    };
    let (w, h, bits) = (w as i32, h as i32, bits as usize);
    // BI_RGB, BI_BITFIELDS, BI_ALPHABITFIELDS: plain rows. RLE and embedded JPEG/PNG: not rewritten.
    if !matches!(compression, 0 | 3 | 6) || ![1, 2, 4, 8, 16, 24, 32].contains(&bits) || w <= 0 || h == 0 {
        return Ok(None);
    }
    let (w, top_down, h) = (w as usize, h < 0, h.unsigned_abs() as usize);
    let stride = (w * bits).div_ceil(32) * 4;
    let offset = offset as usize;
    if offset + stride * h != bytes.len() {
        return Ok(None);
    }
    // Rows top to bottom.
    let stored = &bytes[offset..];
    let rows: Vec<u8> = if top_down {
        stored.to_vec()
    } else {
        (0..h).rev().flat_map(|y| &stored[y * stride..(y + 1) * stride]).copied().collect()
    };
    let new_stride = |nw: usize| (nw * bits).div_ceil(32) * 4;
    let (pixels, nw, nh) = apply_rows(&rows, stride, w, h, bits, op, new_stride);
    let ns = new_stride(nw);

    let mut out = bytes[..offset].to_vec();
    out[18..22].copy_from_slice(&(nw as i32).to_le_bytes());
    out[22..26].copy_from_slice(&(if top_down { -(nh as i32) } else { nh as i32 }).to_le_bytes());
    out[34..38].copy_from_slice(&((ns * nh) as u32).to_le_bytes());
    if op.quarter() {
        // Pixels per metre, x and y.
        let ppm = out[38..46].to_vec();
        out[38..42].copy_from_slice(&ppm[4..8]);
        out[42..46].copy_from_slice(&ppm[0..4]);
    }
    if top_down {
        out.extend_from_slice(&pixels);
    } else {
        for y in (0..nh).rev() {
            out.extend_from_slice(&pixels[y * ns..(y + 1) * ns]);
        }
    }
    let size = out.len() as u32;
    out[2..6].copy_from_slice(&size.to_le_bytes());
    Ok(Some(out))
}
