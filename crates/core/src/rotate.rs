//! Turning images losslessly in the files themselves (the user's R / Shift+R). Where a format allows it, the
//! pixels turn, so every program shows the result, with no generation loss:
//!
//! - JPEG: in the DCT domain (libjpeg-turbo's lossless transform, perfect transforms only: one that would have
//!   to drop or smear partial edge blocks is refused).
//! - PNG, BMP, still GIF, lossless WebP: decoded, turned, re-encoded losslessly (`rewrite`). Only the pixel data
//!   and the size fields change; every other chunk or block (ICC profile, EXIF, XMP, text, comments) is copied as
//!   it was.
//!
//! Otherwise the EXIF Orientation tag carries the turn (TIFF, lossy or animated WebP, animated PNG, and a JPEG
//! whose size doesn't allow a perfect transform), and as the last resort the catalog does (animated GIF,
//! compressed BMP): nothing in the file can.
//!
//! The image a viewer shows is the stored pixels under the EXIF orientation. Turning the pixels applies that
//! orientation and the new turn together, then resets the tag to 1, so the result shows the same in programs that
//! ignore the tag.

use anyhow::Result;

use crate::container::{png_chunks, webp_chunks};
use crate::format::Format;
use crate::metadata;
use crate::orientation;
use crate::rewrite::{self, Op};

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
    let shown = metadata::orientation(bytes, format);
    // The transform that takes the stored pixels to the new upright image.
    let total = orientation::compose(shown, turns % 4);
    let tag = || Outcome::Orientation(total);
    let rewritten = |out: Vec<u8>| -> Outcome {
        // Pixels now upright: the tag (if any) goes back to 1.
        Outcome::Rewritten(metadata::patch_orientation(&out, format, 1).unwrap_or(out))
    };
    Ok(match format {
        Format::Jpeg => match rewrite::jpeg(bytes, Op::Orient(total)) {
            Ok(out) => rewritten(out),
            Err(_) => tag(), // not a perfect transform at this size
        },
        Format::Png if png_chunks(bytes).any(|c| &c.kind == b"acTL") => tag(),
        Format::Png => rewritten(rewrite::png(bytes, Op::Orient(total))?),
        Format::Webp
            if webp_chunks(bytes).any(|c| &c.kind == b"VP8L") && !webp_chunks(bytes).any(|c| &c.kind == b"ANIM") =>
        {
            rewritten(rewrite::webp(bytes, Op::Orient(total))?)
        }
        Format::Webp | Format::Tiff => tag(),
        Format::Gif => match rewrite::gif(bytes, Op::Orient(total))? {
            Some(out) => Outcome::Rewritten(out),
            None => Outcome::Catalog,
        },
        Format::Bmp => match rewrite::bmp(bytes, Op::Orient(total))? {
            Some(out) => Outcome::Rewritten(out),
            None => Outcome::Catalog,
        },
    })
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;
    use crate::decode::{Decoder, MinSize, orient};
    use crate::rewrite::{pack, png_chunk};

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
        let d = crate::display::decode(&mut Decoder::new().unwrap(), bytes, MinSize::Full, 0).unwrap();
        (d.image.width, d.image.height, d.image.pixels)
    }

    /// Expected: the original as shown, turned clockwise `turns` times.
    fn expect_turned(original: &[u8], turns: u8) -> (usize, usize, Vec<u8>) {
        let (w, h, pixels) = shown(original);
        let img = orient(crate::decode::Rgb { width: w, height: h, pixels }, orientation::compose(1, turns));
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
                assert_eq!(metadata::orientation(&out, format), 1);
                assert_eq!(
                    shown(&out),
                    expect_turned(original, turns),
                    "{format:?}, orientation {}, {turns} turns",
                    metadata::orientation(original, format)
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
            let o = metadata::orientation(&aligned, Format::Jpeg);
            for turns in 1..=3 {
                let Outcome::Rewritten(out) = rotate(&aligned, Format::Jpeg, turns).unwrap() else {
                    panic!("aligned JPEG")
                };
                assert_eq!(metadata::orientation(&out, Format::Jpeg), 1);
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
