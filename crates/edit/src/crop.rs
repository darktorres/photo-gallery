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
//!
//! [`cut`] is the cut of an original (`pg_crop::cut`), which must be exact on every format: a JPEG is re-encoded exactly
//! at the quality its tables say, and a lossy WebP stays lossy. Those two come out without their metadata, which the
//! caller copies over. Every other format is cut as [`crop`] cuts it.

use anyhow::{Context, Result, bail, ensure};

use crate::rewrite::chunk::{png_animated, webp_animated, webp_is_lossless};
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

/// Cut `cut` (left, top, right, bottom, stored pixels) off a still image of stored size `width` × `height`, for the
/// originals (`pg_crop::cut`): exact in every format. A JPEG is re-encoded at the quality its luminance table gives
/// (`pg_codec::encode::jpeg_quality`), in its own chroma subsampling; a lossy WebP is encoded lossy again at
/// `pg_codec::encode::WEBP_LOSSY_QUALITY`. Both leave the metadata out, as the other re-encodes do. Every other format
/// is cut as [`crop`] cuts it, so the copies and the cuts of those are the same bytes.
pub fn cut(bytes: &[u8], format: Format, width: u32, height: u32, cut: [u32; 4]) -> Result<Cropped> {
    let [l, t, r, b] = cut;
    ensure!(l + r < width && t + b < height, "the cut {cut:?} leaves nothing of {width}×{height}");
    let rect = [l, t, width - l - r, height - t - b];
    match format {
        Format::Jpeg => {
            let quality = pg_codec::encode::jpeg_quality(bytes)
                .context("the JPEG has no luminance table to read its quality from")?;
            let bytes = pg_codec::encode::jpeg_cropped(bytes, rect, i32::from(quality))?;
            Ok(Cropped { bytes, cut, metadata: false })
        }
        Format::Webp if !webp_animated(bytes) && !webp_is_lossless(bytes) => {
            let bytes = pg_codec::encode::webp_lossy_cropped(bytes, rect, pg_codec::encode::WEBP_LOSSY_QUALITY)?;
            Ok(Cropped { bytes, cut, metadata: false })
        }
        _ => crop(bytes, format, width, height, cut),
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

    /// Photo-like content: blocks of hashed colours with fine noise (as the library's cut tests have). A lossy encode
    /// has detail to lose in it; a smooth gradient is one a lossless encode keeps smaller.
    fn photo(w: usize, h: usize) -> Vec<u8> {
        let hash =
            |a: u32, b: u32| (a.wrapping_mul(2_654_435_761) ^ b.wrapping_mul(40_503)).wrapping_mul(2_246_822_519);
        (0..w * h)
            .flat_map(|i| {
                let (x, y) = (u32::try_from(i % w).unwrap(), u32::try_from(i / w).unwrap());
                let (block, noise) = (hash(x / 6, y / 6), hash(x, y) % 24);
                [0, 8, 16].map(|shift| u8::try_from(60 + (block >> shift) % 150 + noise).unwrap_or(u8::MAX))
            })
            .collect()
    }

    /// The mean and the worst absolute difference between two pixel buffers.
    fn differences(a: &[u8], b: &[u8]) -> (f64, u8) {
        let diff: Vec<u8> = a.iter().zip(b).map(|(x, y)| x.abs_diff(*y)).collect();
        let mean = diff.iter().map(|&d| f64::from(d)).sum::<f64>() / f64::from(u32::try_from(diff.len()).unwrap());
        (mean, diff.iter().copied().max().unwrap())
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
                assert!(pg_container::webp_chunks(&out.bytes).any(|c| &c.kind == b"EXIF"));
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

    /// The cut of a JPEG is exact (no MCU grid: the left and top cuts stay where they were), at the quality the file was
    /// made at, with its subsampling; its pixels are the source's cut, to the loss of one more encode.
    #[test]
    fn jpeg_cut_is_exact_at_the_source_quality() {
        let (w, h) = (100, 70);
        let (sw, sh) = (u32::try_from(w).unwrap(), u32::try_from(h).unwrap());
        let pixels = photo(w, h);
        for (quality, subsamp) in [(95, turbojpeg::Subsamp::Sub2x2), (60, turbojpeg::Subsamp::None)] {
            let mut c = turbojpeg::Compressor::new().unwrap();
            c.set_quality(quality).unwrap();
            c.set_subsamp(subsamp).unwrap();
            let image = turbojpeg::Image {
                pixels: &pixels[..],
                width: w,
                pitch: w * 3,
                height: h,
                format: turbojpeg::PixelFormat::RGB,
            };
            let file = c.compress_to_vec(image).unwrap();
            // Left 5, top 16, right 3, bottom 9: the DCT crop would move the left and top to 16.
            let borders = [5, 16, 3, 9];
            let out = cut(&file, Format::Jpeg, sw, sh, borders).unwrap();
            assert_eq!(out.cut, borders, "exact: nothing moved to a grid");
            assert!(!out.metadata, "re-encoded: the caller copies the metadata");
            let (cw, ch, after) = stored(&out.bytes);
            assert_eq!((cw, ch), (w - 8, h - 25));
            let header = turbojpeg::read_header(&out.bytes).unwrap();
            assert_eq!(header.subsamp, subsamp, "the source's subsampling");
            assert_eq!(pg_codec::encode::jpeg_quality(&out.bytes), Some(u8::try_from(quality).unwrap()));
            let (_, _, before) = stored(&file);
            let want = cut_out(&before, w, borders.map(num::index), h);
            // The same crop one pixel further in is a worse match: the cut is where the borders are.
            let shifted = cut_out(&before, w, [6, 16, 2, 9], h);
            let (mean, worst) = differences(&after, &want);
            let (shifted_mean, _) = differences(&after, &shifted);
            assert!(mean < shifted_mean, "{quality}: exact {mean:.2}, one pixel off {shifted_mean:.2}");
            // Measured: 5.7 (worst 55) at 95 with 4:2:0, 8.6 (worst 90) at 60 with 4:4:4, the loss of one more encode.
            assert!(mean < 10.0 && worst <= 100, "{quality}: mean {mean:.2}, worst {worst}");
        }
    }

    /// A lossy WebP stays lossy: the cut is smaller than the lossless encoding of the same pixels, and it decodes to the
    /// source's cut pixels, to the loss of the encode.
    #[test]
    fn lossy_webp_cut_stays_lossy_and_keeps_its_pixels() {
        let (w, h) = (64, 48);
        let (sw, sh) = (u32::try_from(w).unwrap(), u32::try_from(h).unwrap());
        let pixels = photo(w, h);
        let file = pg_codec::fixtures::webp_lossy_rgb(sw, sh, &pixels, 75.0).unwrap();
        assert!(pg_container::webp_chunks(&file).any(|c| &c.kind == b"VP8 "), "the source is lossy");
        let borders = [3, 5, 7, 2];
        let out = cut(&file, Format::Webp, sw, sh, borders).unwrap();
        assert_eq!(out.cut, borders);
        assert!(!out.metadata);
        assert_eq!(Format::sniff(&out.bytes), Some(Format::Webp));
        assert!(pg_container::webp_chunks(&out.bytes).any(|c| &c.kind == b"VP8 "), "the cut is lossy");
        let (cw, ch) = (w - 10, h - 7);
        let rgb = cut_out(&pixels, w, borders.map(num::index), h);
        let mut lossless = Vec::new();
        image_webp::WebPEncoder::new(&mut lossless)
            .encode(&rgb, u32::try_from(cw).unwrap(), u32::try_from(ch).unwrap(), image_webp::ColorType::Rgb8)
            .unwrap();
        let (dw, dh, got) = stored(&out.bytes);
        assert_eq!((dw, dh), (cw, ch));
        let (_, _, before) = stored(&file);
        let want = cut_out(&before, w, borders.map(num::index), h);
        assert!(out.bytes.len() < lossless.len(), "lossy {} bytes, lossless {}", out.bytes.len(), lossless.len());
        // Measured: 7.1 (worst 39) at quality 90 on this content.
        let (mean, worst) = differences(&got, &want);
        assert!(mean < 10.0 && worst <= 60, "mean {mean:.2}, worst {worst}");
    }

    /// Every other format is cut as the copies are cut: the same bytes from `cut` as from `crop`.
    #[test]
    fn the_other_formats_cut_as_the_copies_do() {
        let (w, h) = (37, 23);
        let (sw, sh) = (u32::try_from(w).unwrap(), u32::try_from(h).unwrap());
        let rgb = pattern(w, h);
        let borders = [3, 5, 7, 2];
        let mut webp = Vec::new();
        image_webp::WebPEncoder::new(&mut webp).encode(&rgb, sw, sh, image_webp::ColorType::Rgb8).unwrap();
        for (format, file) in [
            (Format::Png, pg_testkit::png_rgb(sw, sh, &rgb).unwrap()),
            (Format::Bmp, pg_testkit::bmp_rgb(sw, sh, &rgb).unwrap()),
            (Format::Tiff, pg_testkit::tiff_rgb(sw, sh, &rgb).unwrap()),
            (Format::Webp, webp),
        ] {
            let cut_bytes = cut(&file, format, sw, sh, borders).unwrap();
            let copy = crop(&file, format, sw, sh, borders).unwrap();
            assert_eq!(cut_bytes.bytes, copy.bytes, "{format:?}");
            assert_eq!(cut_bytes.metadata, copy.metadata, "{format:?}");
        }
    }
}
