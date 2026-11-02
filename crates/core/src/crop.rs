//! Cropping a file's borders off (`detection::Detection`'s cut, on the stored pixels), into new bytes in the file's
//! own format; the caller writes them to a copy, never over the original. Cropping the stored pixels keeps the EXIF
//! orientation valid.
//!
//! - JPEG: losslessly, in the DCT domain. A lossless crop starts on the MCU grid (8 or 16 px), so the left and top
//!   cuts move inward to it, cutting up to 15 px more there (as `jpegtran -crop`); the right and bottom are exact.
//! - PNG, BMP, still GIF, WebP: exact, re-encoded losslessly with every other chunk kept (`rewrite`). A lossy WebP
//!   comes out lossless: larger, with no generation loss.
//! - TIFF and compressed BMP: exact, re-encoded (TIFF losslessly) without their metadata; [`Cropped::metadata`]
//!   is false, and the caller copies it over (ExifTool).

use std::io::Cursor;

use anyhow::{Context, Result, bail, ensure};

use crate::container::{png_chunks, webp_chunks};
use crate::format::Format;
use crate::rewrite::{self, Op};

pub struct Cropped {
    pub bytes: Vec<u8>,
    /// What was cut from the left, top, right and bottom (stored pixels): the cut asked for, but for JPEG's left
    /// and top, moved inward to the MCU grid.
    pub cut: [u32; 4],
    /// The file's metadata came along (otherwise only the pixels did).
    pub metadata: bool,
}

/// Crop `cut` (left, top, right, bottom, stored pixels) off a still image of stored size `width` × `height`.
pub fn crop(bytes: &[u8], format: Format, width: u32, height: u32, cut: [u32; 4]) -> Result<Cropped> {
    let [l, t, r, b] = cut;
    ensure!(l + r < width && t + b < height, "the cut {cut:?} leaves nothing of {width}×{height}");
    let exact =
        |l: u32, t: u32| Op::Crop([l as usize, t as usize, (width - l - r) as usize, (height - t - b) as usize]);
    let kept = |bytes: Vec<u8>| Ok(Cropped { bytes, cut, metadata: true });
    match format {
        Format::Jpeg => {
            let (mw, mh) = rewrite::jpeg_mcu(bytes)?;
            let (l, t) = (l.next_multiple_of(mw as u32), t.next_multiple_of(mh as u32));
            ensure!(l + r < width && t + b < height, "the cut leaves nothing once on the JPEG's {mw}×{mh} grid");
            let out = rewrite::jpeg(bytes, exact(l, t))?;
            let header = turbojpeg::read_header(&out)?;
            let want = ((width - l - r) as usize, (height - t - b) as usize);
            ensure!((header.width, header.height) == want, "the crop came out {}×{}", header.width, header.height);
            Ok(Cropped { bytes: out, cut: [l, t, r, b], metadata: true })
        }
        Format::Png if png_chunks(bytes).any(|c| &c.kind == b"acTL") => bail!("an animated PNG isn't cropped"),
        Format::Png => kept(rewrite::png(bytes, exact(l, t))?),
        Format::Webp if webp_chunks(bytes).any(|c| &c.kind == b"ANIM") => bail!("an animated WebP isn't cropped"),
        Format::Webp => kept(rewrite::webp(bytes, exact(l, t))?),
        Format::Gif => match rewrite::gif(bytes, exact(l, t))? {
            Some(out) => kept(out),
            None => bail!("an animated GIF isn't cropped"),
        },
        Format::Bmp => match rewrite::bmp(bytes, exact(l, t))? {
            Some(out) => kept(out),
            None => reencoded(bytes, image::ImageFormat::Bmp, l, t, width - l - r, height - t - b, cut),
        },
        Format::Tiff => reencoded(bytes, image::ImageFormat::Tiff, l, t, width - l - r, height - t - b, cut),
    }
}

/// Decoded, cropped and encoded again in the same format, without the metadata.
fn reencoded(
    bytes: &[u8],
    format: image::ImageFormat,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    cut: [u32; 4],
) -> Result<Cropped> {
    let img = image::load_from_memory_with_format(bytes, format).context("decoding")?;
    let mut out = Cursor::new(Vec::new());
    img.crop_imm(x, y, w, h).write_to(&mut out, format).context("encoding")?;
    Ok(Cropped { bytes: out.into_inner(), cut, metadata: false })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decode::{Decoder, MinSize};

    fn pattern(w: usize, h: usize) -> Vec<u8> {
        (0..w * h)
            .flat_map(|i| {
                let (x, y) = (i % w, i / w);
                [(x * 255 / w) as u8, (y * 255 / h) as u8, ((x * 7 + y * 13) % 256) as u8]
            })
            .collect()
    }

    /// Decoded stored pixels, full size.
    fn stored(bytes: &[u8]) -> (usize, usize, Vec<u8>) {
        let d = crate::display::decode_stored(&mut Decoder::new().unwrap(), bytes, MinSize::Full, 0).unwrap();
        (d.image.width, d.image.height, d.image.pixels)
    }

    fn cut_out(pixels: &[u8], w: usize, [l, t, r, b]: [usize; 4], h: usize) -> Vec<u8> {
        (t..h - b).flat_map(|y| pixels[(y * w + l) * 3..(y * w + w - r) * 3].to_vec()).collect()
    }

    #[test]
    fn lossless_formats_crop_exactly_and_keep_their_chunks() {
        let (w, h) = (37, 23);
        let rgb = pattern(w, h);
        let img = image::RgbImage::from_raw(w as u32, h as u32, rgb.clone()).unwrap();
        let cut = [3, 5, 7, 2];
        let mut files = Vec::new();
        for format in [image::ImageFormat::Png, image::ImageFormat::Bmp, image::ImageFormat::Tiff] {
            let mut file = Cursor::new(Vec::new());
            img.write_to(&mut file, format).unwrap();
            files.push(file.into_inner());
        }
        let mut webp = Vec::new();
        let mut e = image_webp::WebPEncoder::new(&mut webp);
        e.set_exif_metadata(b"II*\0\x08\0\0\0\0\0\0\0\0\0".to_vec());
        e.encode(&rgb, w as u32, h as u32, image_webp::ColorType::Rgb8).unwrap();
        files.push(webp);
        // BigTIFF, which `image` doesn't write: the tiff crate's encoder does.
        files.push(crate::decode::tiff_rgb8(w as u32, h as u32, &rgb, true));
        for file in files {
            let format = Format::sniff(&file).unwrap();
            let out = crop(&file, format, w as u32, h as u32, cut).unwrap();
            assert_eq!(Format::sniff(&out.bytes), Some(format));
            assert_eq!(out.cut, cut);
            assert_eq!(out.metadata, format != Format::Tiff, "{format:?}");
            let (cw, ch, pixels) = stored(&out.bytes);
            assert_eq!((cw, ch), (w - 10, h - 7), "{format:?}");
            assert_eq!(pixels, cut_out(&rgb, w, cut.map(|v| v as usize), h), "{format:?}");
            if format == Format::Webp {
                assert!(webp_chunks(&out.bytes).any(|c| &c.kind == b"EXIF"));
            }
        }
    }

    #[test]
    fn gif_crops_the_frame_and_the_screen() {
        let (w, h) = (19, 11);
        let img = image::RgbImage::from_raw(w as u32, h as u32, pattern(w, h)).unwrap();
        let mut file = Cursor::new(Vec::new());
        img.write_to(&mut file, image::ImageFormat::Gif).unwrap();
        let file = file.into_inner();
        let (_, _, before) = stored(&file);
        let cut = [2, 1, 4, 3];
        let out = crop(&file, Format::Gif, w as u32, h as u32, cut).unwrap();
        let (cw, ch, after) = stored(&out.bytes);
        assert_eq!((cw, ch), (w - 6, h - 4));
        assert_eq!(after, cut_out(&before, w, cut.map(|v| v as usize), h));
    }

    #[test]
    fn jpeg_crops_losslessly_on_the_mcu_grid() {
        let (w, h) = (100, 70);
        let pixels = pattern(w, h);
        let mut c = turbojpeg::Compressor::new().unwrap();
        c.set_quality(95).unwrap();
        c.set_subsamp(turbojpeg::Subsamp::Sub2x2).unwrap();
        let image = turbojpeg::Image {
            pixels: &pixels[..],
            width: w,
            pitch: w * 3,
            height: h,
            format: turbojpeg::PixelFormat::RGB,
        };
        let file = c.compress_to_vec(image).unwrap();
        // 16×16 MCUs: left 5 → 16, top 16 stays; right and bottom exact.
        let out = crop(&file, Format::Jpeg, w as u32, h as u32, [5, 16, 3, 9]).unwrap();
        assert_eq!(out.cut, [16, 16, 3, 9]);
        let (cw, ch, after) = stored(&out.bytes);
        assert_eq!((cw, ch), (81, 45));
        // The same coefficients: decoding the crop gives the original's decoded pixels there, but for the 2 px at
        // the new edges, where the chroma upsampling has other neighbours.
        let (_, _, before) = stored(&file);
        let want = cut_out(&before, w, [16, 16, 3, 9], h);
        let inner = |p: &[u8]| cut_out(p, cw, [2, 2, 2, 2], ch);
        let worst = inner(&after).iter().zip(&inner(&want)).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
        assert!(worst <= 1, "max difference {worst}");
        assert!(crop(&file, Format::Jpeg, w as u32, h as u32, [60, 0, 40, 0]).is_err());
    }
}
