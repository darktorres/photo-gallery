//! Turning images losslessly in the files themselves (the user's R / Shift+R). Where a format allows it, the
//! pixels turn, so every program shows the result, with no generation loss:
//!
//! - JPEG: in the DCT domain (libjpeg-turbo's lossless transform, perfect transforms only: one that would have
//!   to drop or smear partial edge blocks is refused).
//! - PNG, BMP, still GIF, lossless WebP: decoded, turned, re-encoded losslessly. Only the pixel data and the size
//!   fields change; every other chunk or block (ICC profile, EXIF, XMP, text, comments) is copied as it was.
//!
//! Otherwise the EXIF Orientation tag carries the turn (TIFF, lossy or animated WebP, animated PNG, and a JPEG
//! whose size doesn't allow a perfect transform), and as the last resort the catalog does (animated GIF,
//! compressed BMP): nothing in the file can.
//!
//! The image a viewer shows is the stored pixels under the EXIF orientation. Turning the pixels applies that
//! orientation and the new turn together, then resets the tag to 1, so the result shows the same in programs that
//! ignore the tag.

use std::io::Cursor;

use anyhow::{Context, Result, bail};
use turbojpeg::{Transform, TransformOp, Transformer};

use crate::decode::{Format, exif_orientation, transform};
use crate::metadata::{self, GifBlock, compose_orientation, gif_blocks, gif_header_len, png_chunks, webp_chunks};

/// What turning a file comes to.
#[derive(Debug)]
pub enum Outcome {
    /// The new file: pixels turned, orientation reset.
    Rewritten(Vec<u8>),
    /// Set the EXIF Orientation tag to this (metadata::patch_orientation, or ExifTool when the file has no tag).
    Orientation(u8),
    /// Nothing in the file can carry it: keep the turn in the catalog.
    Catalog,
}

/// Turn a file `turns` quarter turns clockwise (as shown).
pub fn rotate(bytes: &[u8], format: Format, turns: u8) -> Result<Outcome> {
    let shown = exif_orientation(bytes);
    // The transform that takes the stored pixels to the new upright image.
    let total = compose_orientation(shown, turns % 4);
    let tag = || Outcome::Orientation(total);
    let rewritten = |out: Vec<u8>| -> Outcome {
        // Pixels now upright: the tag (if any) goes back to 1.
        Outcome::Rewritten(metadata::patch_orientation(&out, format, 1).unwrap_or(out))
    };
    Ok(match format {
        Format::Jpeg => match jpeg(bytes, total) {
            Ok(out) => rewritten(out),
            Err(_) => tag(), // not a perfect transform at this size
        },
        Format::Png if png_chunks(bytes).any(|c| &c.kind == b"acTL") => tag(),
        Format::Png => rewritten(png(bytes, total)?),
        Format::Webp
            if webp_chunks(bytes).any(|c| &c.kind == b"VP8L") && !webp_chunks(bytes).any(|c| &c.kind == b"ANIM") =>
        {
            rewritten(webp_lossless(bytes, total)?)
        }
        Format::Webp | Format::Tiff => tag(),
        Format::Gif => match gif(bytes, total)? {
            Some(out) => Outcome::Rewritten(out),
            None => Outcome::Catalog,
        },
        Format::Bmp => match bmp(bytes, total)? {
            Some(out) => Outcome::Rewritten(out),
            None => Outcome::Catalog,
        },
    })
}

fn quarter(o: u8) -> bool {
    matches!(o, 5..=8)
}

/// Where a pixel of a `w` × `h` raster lands under orientation `o` (the inverse of `decode::transform`'s lookup).
fn map_point(o: u8, x: usize, y: usize, w: usize, h: usize) -> (usize, usize) {
    match o {
        2 => (w - 1 - x, y),
        3 => (w - 1 - x, h - 1 - y),
        4 => (x, h - 1 - y),
        5 => (y, x),
        6 => (h - 1 - y, x),
        7 => (h - 1 - y, w - 1 - x),
        8 => (y, w - 1 - x),
        _ => (x, y),
    }
}

// --- JPEG -------------------------------------------------------------------------------------------------------

fn jpeg(bytes: &[u8], o: u8) -> Result<Vec<u8>> {
    let op = match o {
        2 => TransformOp::Hflip,
        3 => TransformOp::Rot180,
        4 => TransformOp::Vflip,
        5 => TransformOp::Transpose,
        6 => TransformOp::Rot90,
        7 => TransformOp::Transverse,
        8 => TransformOp::Rot270,
        _ => return Ok(bytes.to_vec()),
    };
    // Keep the entropy coding kind: progressive stays progressive; baseline gets optimized Huffman tables (the
    // transform can't reuse the original ones).
    let progressive = jpeg_is_progressive(bytes);
    let mut t = Transform::default();
    t.op = op;
    t.perfect = true;
    t.progressive = progressive;
    t.optimize = !progressive;
    Ok(Transformer::new()?.transform_to_vec(&t, bytes)?)
}

fn jpeg_is_progressive(bytes: &[u8]) -> bool {
    // SOF2 (progressive Huffman) among the markers before the scan.
    let mut i = 2;
    while i + 4 <= bytes.len() && bytes[i] == 0xFF {
        let marker = bytes[i + 1];
        if marker == 0xC2 {
            return true;
        }
        if marker == 0xDA {
            return false;
        }
        i += 2 + u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
    }
    false
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

fn pack(values: &[u8], stride: usize, w: usize, h: usize, bits: usize) -> Vec<u8> {
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

/// Turn a raster of rows `stride` bytes apart, `bits` per pixel, under orientation `o`; the result's rows are
/// `new_stride(w')` bytes apart. Returns it with the new width and height.
fn turn_rows(
    rows: &[u8],
    stride: usize,
    w: usize,
    h: usize,
    bits: usize,
    o: u8,
    new_stride: impl Fn(usize) -> usize,
) -> (Vec<u8>, usize, usize) {
    if bits < 8 {
        let (values, nw, nh) = transform(&unpack(rows, stride, w, h, bits), w, h, 1, o);
        let s = new_stride(nw);
        return (pack(&values, s, nw, nh, bits), nw, nh);
    }
    let unit = bits / 8;
    let packed: Vec<u8> = (0..h).flat_map(|y| &rows[y * stride..y * stride + w * unit]).copied().collect();
    let (pixels, nw, nh) = transform(&packed, w, h, unit, o);
    let s = new_stride(nw);
    let mut out = vec![0u8; s * nh];
    for y in 0..nh {
        out[y * s..y * s + nw * unit].copy_from_slice(&pixels[y * nw * unit..(y + 1) * nw * unit]);
    }
    (out, nw, nh)
}

// --- PNG --------------------------------------------------------------------------------------------------------

fn png_chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 12);
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    out.extend_from_slice(&crc32fast::hash(&out[4..]).to_be_bytes());
    out
}

fn png(bytes: &[u8], o: u8) -> Result<Vec<u8>> {
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::IDENTITY);
    let mut reader = decoder.read_info()?;
    let info = reader.info().clone();
    let mut raw = vec![0u8; reader.output_buffer_size().context("PNG too large")?];
    let frame = reader.next_frame(&mut raw)?;
    let (w, h) = (info.width as usize, info.height as usize);
    let bits = info.color_type.samples() * info.bit_depth as usize;
    let (pixels, nw, nh) = turn_rows(&raw, frame.line_size, w, h, bits, o, |nw| (nw * bits).div_ceil(8));

    // Encode the turned pixels in the same color type and depth, and keep only the image data from it.
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
            b"pHYs" if quarter(o) && c.data.len() == 9 => {
                let mut phys = c.data.to_vec();
                phys[..8].rotate_left(4); // pixels per unit, x and y
                out.extend(png_chunk(b"pHYs", &phys));
            }
            _ => out.extend_from_slice(raw_chunk),
        }
    }
    Ok(out)
}

// --- WebP (lossless) --------------------------------------------------------------------------------------------

fn webp_lossless(bytes: &[u8], o: u8) -> Result<Vec<u8>> {
    let mut decoder = image_webp::WebPDecoder::new(Cursor::new(bytes))?;
    let (w, h) = decoder.dimensions();
    let alpha = decoder.has_alpha();
    let mut rgba = vec![0u8; decoder.output_buffer_size().context("WebP too large")?];
    decoder.read_image(&mut rgba)?;
    let unit = if alpha { 4 } else { 3 };
    let (pixels, nw, nh) = transform(&rgba, w as usize, h as usize, unit, o);
    let mut encoded = Vec::new();
    let color = if alpha { image_webp::ColorType::Rgba8 } else { image_webp::ColorType::Rgb8 };
    image_webp::WebPEncoder::new(&mut encoded).encode(&pixels, nw as u32, nh as u32, color)?;
    let vp8l = webp_chunks(&encoded).find(|c| &c.kind == b"VP8L").context("encoder wrote no VP8L chunk")?;
    let vp8l = encoded[vp8l.at..vp8l.at + vp8l.len].to_vec();

    let mut body = b"WEBP".to_vec();
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
            b"VP8L" => body.extend_from_slice(&vp8l),
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

/// A still GIF turned; `None` for an animation (more than one image), which keeps its turn in the catalog.
fn gif(bytes: &[u8], o: u8) -> Result<Option<Vec<u8>>> {
    let blocks = gif_blocks(bytes).context("damaged GIF")?;
    let images: Vec<_> = blocks.iter().filter(|b| matches!(b, GifBlock::Image { .. })).collect();
    let [GifBlock::Image { range }] = images[..] else { return Ok(None) };
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
    let (turned, nw, nh) = transform(&indices, w, h, 1, o);
    let data = weezl::encode::Encoder::new(weezl::BitOrder::Lsb, min_code).encode(&turned)?;

    // The frame's place on the logical screen turns with it.
    let (a, b) = (map_point(o, left, top, sw, sh), map_point(o, left + w - 1, top + h - 1, sw, sh));
    let (nleft, ntop) = (a.0.min(b.0), a.1.min(b.1));
    let (nsw, nsh) = if quarter(o) { (sh, sw) } else { (sw, sh) };

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

/// An uncompressed BMP turned; `None` for what this doesn't rewrite (RLE compression, OS/2 headers, data after
/// the pixels), which keeps its turn in the catalog.
fn bmp(bytes: &[u8], o: u8) -> Result<Option<Vec<u8>>> {
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
    let (turned, nw, nh) = turn_rows(&rows, stride, w, h, bits, o, new_stride);
    let ns = new_stride(nw);

    let mut out = bytes[..offset].to_vec();
    out[18..22].copy_from_slice(&(nw as i32).to_le_bytes());
    out[22..26].copy_from_slice(&(if top_down { -(nh as i32) } else { nh as i32 }).to_le_bytes());
    out[34..38].copy_from_slice(&((ns * nh) as u32).to_le_bytes());
    if quarter(o) {
        // Pixels per metre, x and y.
        let ppm = out[38..46].to_vec();
        out[38..42].copy_from_slice(&ppm[4..8]);
        out[42..46].copy_from_slice(&ppm[0..4]);
    }
    if top_down {
        out.extend_from_slice(&turned);
    } else {
        for y in (0..nh).rev() {
            out.extend_from_slice(&turned[y * ns..(y + 1) * ns]);
        }
    }
    let size = out.len() as u32;
    out[2..6].copy_from_slice(&size.to_le_bytes());
    Ok(Some(out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decode::{Decoder, MinSize, orient};

    /// A test image with distinct, smoothly varying pixels (so a wrong turn can't match by chance).
    fn pattern(w: usize, h: usize) -> Vec<u8> {
        (0..w * h)
            .flat_map(|i| {
                let (x, y) = (i % w, i / w);
                [(x * 255 / w) as u8, (y * 255 / h) as u8, ((x * 7 + y * 13) % 256) as u8]
            })
            .collect()
    }

    /// Decode as shown, full size.
    fn shown(bytes: &[u8]) -> (usize, usize, Vec<u8>) {
        let d = Decoder::new().unwrap().decode(bytes, MinSize::Full, 0).unwrap();
        (d.image.width, d.image.height, d.image.pixels)
    }

    /// Expected: the original as shown, turned clockwise `turns` times.
    fn expect_turned(original: &[u8], turns: u8) -> (usize, usize, Vec<u8>) {
        let (w, h, pixels) = shown(original);
        let img = orient(crate::decode::Rgb { width: w, height: h, pixels }, compose_orientation(1, turns));
        (img.width, img.height, img.pixels)
    }

    /// A minimal EXIF block (TIFF, little-endian) holding only an orientation.
    fn exif_tiff(o: u8) -> Vec<u8> {
        let mut t = b"II*\0\x08\0\0\0".to_vec();
        t.extend(1u16.to_le_bytes());
        t.extend(0x0112u16.to_le_bytes());
        t.extend(3u16.to_le_bytes());
        t.extend(1u32.to_le_bytes());
        t.extend((o as u16).to_le_bytes());
        t.extend([0, 0]);
        t.extend(0u32.to_le_bytes());
        t
    }

    /// The file with an EXIF orientation tag added (JPEG APP1, PNG eXIf).
    fn tagged(file: &[u8], format: Format, o: u8) -> Vec<u8> {
        match format {
            Format::Jpeg => {
                let mut seg = b"Exif\0\0".to_vec();
                seg.extend(exif_tiff(o));
                let mut out = file[..2].to_vec();
                out.extend([0xFF, 0xE1]);
                out.extend(((seg.len() + 2) as u16).to_be_bytes());
                out.extend(seg);
                out.extend_from_slice(&file[2..]);
                out
            }
            Format::Png => {
                let mut out = file[..33].to_vec(); // signature and IHDR
                out.extend(png_chunk(b"eXIf", &exif_tiff(o)));
                out.extend_from_slice(&file[33..]);
                out
            }
            _ => unreachable!(),
        }
    }

    /// Every turn of every starting orientation gives exactly the original as shown, turned; the result shows
    /// upright with orientation 1.
    fn check_exact(variants: &[Vec<u8>], format: Format) {
        for original in variants {
            for turns in 1..=3 {
                let Outcome::Rewritten(out) = rotate(original, format, turns).unwrap() else {
                    panic!("{format:?} should be rewritten")
                };
                assert_eq!(Format::sniff(&out), Some(format));
                assert_eq!(exif_orientation(&out), 1);
                assert_eq!(
                    shown(&out),
                    expect_turned(original, turns),
                    "{format:?}, orientation {}, {turns} turns",
                    exif_orientation(original)
                );
            }
        }
    }

    fn all_orientations(file: &[u8], format: Format) -> Vec<Vec<u8>> {
        std::iter::once(file.to_vec()).chain((1..=8).map(|o| tagged(file, format, o))).collect()
    }

    #[test]
    fn png_all_layouts_exact() {
        let (w, h) = (13, 7);
        let rgb = pattern(w, h);
        for (color, depth, data) in [
            (png::ColorType::Rgb, png::BitDepth::Eight, rgb.clone()),
            (png::ColorType::Rgb, png::BitDepth::Sixteen, rgb.iter().flat_map(|&v| [v, v ^ 0x5A]).collect()),
            (
                png::ColorType::Rgba,
                png::BitDepth::Eight,
                rgb.chunks(3).flat_map(|p| [p[0], p[1], p[2], p[0] ^ p[2]]).collect(),
            ),
            (png::ColorType::Grayscale, png::BitDepth::Two, {
                // 2-bit gray: packed four to a byte, rows padded.
                let values: Vec<u8> = (0..w * h).map(|i| (i % 4) as u8).collect();
                pack(&values, (w * 2).div_ceil(8), w, h, 2)
            }),
        ] {
            let mut file = Vec::new();
            let mut e = png::Encoder::new(&mut file, w as u32, h as u32);
            e.set_color(color);
            e.set_depth(depth);
            let mut writer = e.write_header().unwrap();
            writer.write_image_data(&data).unwrap();
            writer.finish().unwrap();
            check_exact(&all_orientations(&file, Format::Png), Format::Png);
        }
    }

    #[test]
    fn bmp_gif_webp_exact() {
        let (w, h) = (11, 6);
        let rgb = pattern(w, h);
        let img = image::RgbImage::from_raw(w as u32, h as u32, rgb.clone()).unwrap();
        for format in [image::ImageFormat::Bmp, image::ImageFormat::Gif] {
            let mut file = Cursor::new(Vec::new());
            img.write_to(&mut file, format).unwrap();
            // No EXIF in these formats: no starting orientation but 1.
            let file = file.into_inner();
            let format = Format::sniff(&file).unwrap();
            check_exact(&[file], format);
        }
        // Lossless WebP, plain and with an EXIF chunk carrying each orientation (VP8X form).
        let mut variants = Vec::new();
        for o in 0..=8 {
            let mut webp = Vec::new();
            let mut e = image_webp::WebPEncoder::new(&mut webp);
            if o > 0 {
                e.set_exif_metadata(exif_tiff(o));
            }
            e.encode(&rgb, w as u32, h as u32, image_webp::ColorType::Rgb8).unwrap();
            variants.push(webp);
        }
        check_exact(&variants, Format::Webp);
    }

    #[test]
    fn jpeg_perfect_or_tag() {
        let jpeg = |w: usize, h: usize| {
            let mut c = turbojpeg::Compressor::new().unwrap();
            c.set_quality(95).unwrap();
            c.set_subsamp(turbojpeg::Subsamp::Sub2x2).unwrap();
            let pixels = pattern(w, h);
            c.compress_to_vec(turbojpeg::Image {
                pixels: &pixels[..],
                width: w,
                pitch: w * 3,
                height: h,
                format: turbojpeg::PixelFormat::RGB,
            })
            .unwrap()
        };
        // MCU-aligned: the pixels turn. Decoding the turned coefficients matches turning the decoded image up
        // to the decoder's chroma rounding.
        for aligned in all_orientations(&jpeg(64, 32), Format::Jpeg) {
            let o = exif_orientation(&aligned);
            for turns in 1..=3 {
                let Outcome::Rewritten(out) = rotate(&aligned, Format::Jpeg, turns).unwrap() else {
                    panic!("aligned JPEG")
                };
                assert_eq!(exif_orientation(&out), 1);
                let (got, want) = (shown(&out), expect_turned(&aligned, turns));
                assert_eq!((got.0, got.1), (want.0, want.1), "orientation {o}, {turns} turns");
                let worst = got.2.iter().zip(&want.2).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
                assert!(worst <= 3, "orientation {o}, {turns} turns: max difference {worst}");
            }
        }
        // Not aligned: no perfect transform, so the orientation tag carries it.
        assert!(matches!(rotate(&jpeg(65, 33), Format::Jpeg, 1).unwrap(), Outcome::Orientation(6)));
    }
}
