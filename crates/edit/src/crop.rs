//! Cropping a file's borders off (`detection::Detection`'s cut, on the stored pixels), into new bytes in the file's
//! own format; the caller writes them to a copy, never over the original. Cropping the stored pixels keeps the EXIF
//! orientation valid.
//!
//! - JPEG: losslessly, in the DCT domain. A lossless crop starts on the MCU grid (8 or 16 px), so the left and top
//!   cuts move inward to it, cutting up to 15 px more there (as `jpegtran -crop`); the right and bottom are exact.
//! - PNG, BMP, still GIF, WebP: exact, re-encoded losslessly with every other chunk kept (`rewrite`). A lossy WebP
//!   comes out lossless: larger, with no generation loss.
//! - TIFF and compressed BMP: exact, re-encoded without their metadata; [`Cropped::metadata`] is false, and the
//!   caller copies it over (ExifTool). An 8- or 16-bit CMYK or RGB TIFF keeps its samples and its kind (classic or
//!   BigTIFF; `pg_codec::encode::cropped`). Not kept for other TIFFs, which go through `image`: they come out
//!   classic, 1-bit grey becomes 8-bit, and palette and YCbCr TIFFs are refused.

use anyhow::{Result, bail, ensure};

use crate::rewrite::chunk::{png_animated, webp_animated};
use crate::rewrite::{self, op::Op};
use pg_model::{format::Format, num};

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
    let exact = |l: u32, t: u32| {
        Op::Crop([num::index(l), num::index(t), num::index(width - l - r), num::index(height - t - b)])
    };
    let kept = |bytes: Vec<u8>| Ok(Cropped { bytes, cut, metadata: true });
    match format {
        Format::Jpeg => {
            let (mw, mh) = rewrite::jpeg::mcu(bytes)?;
            let (l, t) = (l.next_multiple_of(u32::try_from(mw)?), t.next_multiple_of(u32::try_from(mh)?));
            ensure!(l + r < width && t + b < height, "the cut leaves nothing once on the JPEG's {mw}×{mh} grid");
            let out = rewrite::jpeg::rewrite(bytes, exact(l, t))?;
            let header = turbojpeg::read_header(&out)?;
            let want = (num::index(width - l - r), num::index(height - t - b));
            ensure!((header.width, header.height) == want, "the crop came out {}×{}", header.width, header.height);
            Ok(Cropped { bytes: out, cut: [l, t, r, b], metadata: true })
        }
        Format::Png if png_animated(bytes) => bail!("an animated PNG isn't cropped"),
        Format::Png => kept(rewrite::png::rewrite(bytes, exact(l, t))?),
        Format::Webp if webp_animated(bytes) => bail!("an animated WebP isn't cropped"),
        Format::Webp => kept(rewrite::webp::rewrite(bytes, exact(l, t))?),
        Format::Gif => match rewrite::gif::rewrite(bytes, exact(l, t))? {
            Some(out) => kept(out),
            None => bail!("an animated GIF isn't cropped"),
        },
        Format::Bmp => match rewrite::bmp::rewrite(bytes, exact(l, t))? {
            Some(out) => kept(out),
            None => reencoded(bytes, Format::Bmp, l, t, width - l - r, height - t - b, cut),
        },
        Format::Tiff => reencoded(bytes, Format::Tiff, l, t, width - l - r, height - t - b, cut),
    }
}

/// Decoded, cropped and encoded again in the same format, without the metadata.
fn reencoded(bytes: &[u8], format: Format, x: u32, y: u32, w: u32, h: u32, cut: [u32; 4]) -> Result<Cropped> {
    let bytes = pg_codec::encode::cropped(bytes, format, x, y, w, h)?;
    Ok(Cropped { bytes, cut, metadata: false })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::stored;

    fn pattern(w: usize, h: usize) -> Vec<u8> {
        (0..w * h)
            .flat_map(|i| {
                let (x, y) = (i % w, i / w);
                [x * 255 / w, y * 255 / h, (x * 7 + y * 13) % 256].map(|v| u8::try_from(v).unwrap())
            })
            .collect()
    }

    fn cut_out(pixels: &[u8], w: usize, [l, t, r, b]: [usize; 4], h: usize) -> Vec<u8> {
        (t..h - b).flat_map(|y| pixels[(y * w + l) * 3..(y * w + w - r) * 3].to_vec()).collect()
    }

    #[test]
    fn lossless_formats_crop_exactly_and_keep_their_chunks() {
        let (w, h) = (37, 23);
        let (sw, sh) = (u32::try_from(w).unwrap(), u32::try_from(h).unwrap());
        let rgb = pattern(w, h);
        let cut = [3, 5, 7, 2];
        let mut files = vec![
            pg_testkit::png_rgb(sw, sh, &rgb).unwrap(),
            pg_testkit::bmp_rgb(sw, sh, &rgb).unwrap(),
            pg_testkit::tiff_rgb(sw, sh, &rgb).unwrap(),
        ];
        let mut webp = Vec::new();
        let mut e = image_webp::WebPEncoder::new(&mut webp);
        e.set_exif_metadata(b"II*\0\x08\0\0\0\0\0\0\0\0\0".to_vec());
        e.encode(&rgb, sw, sh, image_webp::ColorType::Rgb8).unwrap();
        files.push(webp);
        // BigTIFF, which `image` doesn't write: the tiff crate's encoder does.
        files.push(pg_codec::fixtures::tiff_rgb8(sw, sh, &rgb, true).unwrap());
        for file in files {
            let format = Format::sniff(&file).unwrap();
            let out = crop(&file, format, sw, sh, cut).unwrap();
            assert_eq!(Format::sniff(&out.bytes), Some(format));
            assert_eq!(out.cut, cut);
            assert_eq!(out.metadata, format != Format::Tiff, "{format:?}");
            let (cw, ch, pixels) = stored(&out.bytes);
            assert_eq!((cw, ch), (w - 10, h - 7), "{format:?}");
            assert_eq!(pixels, cut_out(&rgb, w, cut.map(num::index), h), "{format:?}");
            if format == Format::Webp {
                assert!(webp_chunks(&out.bytes).any(|c| &c.kind == b"EXIF"));
            }
        }
    }

    #[test]
    fn gif_crops_the_frame_and_the_screen() {
        let (w, h) = (19, 11);
        let (sw, sh) = (u32::try_from(w).unwrap(), u32::try_from(h).unwrap());
        let file = pg_testkit::gif_rgb(sw, sh, &pattern(w, h)).unwrap();
        let (_, _, before) = stored(&file);
        let cut = [2, 1, 4, 3];
        let out = crop(&file, Format::Gif, sw, sh, cut).unwrap();
        let (cw, ch, after) = stored(&out.bytes);
        assert_eq!((cw, ch), (w - 6, h - 4));
        assert_eq!(after, cut_out(&before, w, cut.map(num::index), h));
    }

    #[test]
    fn jpeg_crops_losslessly_on_the_mcu_grid() {
        let (w, h) = (100, 70);
        let (sw, sh) = (u32::try_from(w).unwrap(), u32::try_from(h).unwrap());
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
        let out = crop(&file, Format::Jpeg, sw, sh, [5, 16, 3, 9]).unwrap();
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
        assert!(crop(&file, Format::Jpeg, sw, sh, [60, 0, 40, 0]).is_err());
    }
}
