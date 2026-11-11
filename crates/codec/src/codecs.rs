//! The decoders for what libjpeg-turbo doesn't decode: PNG, GIF, WebP, BMP and TIFF, and the JPEGs it rejects (CMYK,
//! some damaged files). Each format's crate is driven directly, as `image::load_from_memory_with_format` drives it
//! (`ImageReader::decode`, then the format's `ImageDecoder`, then `DynamicImage::into_rgb8` or `into_rgba8`): the
//! same options, the same limits (`Limits::default()`: 512 MiB, less the decoded size, `ImageReader::decode`) and
//! the same conversions to 8-bit RGB, so every file decodes to the same pixels, or fails with the same `ImageError`
//! (its text is stored as `analysis.error`).
//!
//! What differs is where things go. `image` makes a boxed decoder and a new image buffer for each file, converts
//! into another one, and its TIFF wrapper reads into a buffer of its own before copying into the image. Here the
//! pixels go straight into the caller's buffer when they are 8-bit RGB or RGBA (an RGBA image is then flattened onto
//! white in place), and other sample types are read into [`Codecs::raw`], which is kept from file to file, and
//! converted from there into the caller's buffer in one pass.
//!
//! Each format is a module of free functions over the buffers it uses ([`png`], [`gif`], [`webp`], [`jpeg`],
//! [`tiff`]); they share the conversions ([`convert`]) and the size cap ([`limits`]). This module holds the buffers
//! and picks the format.

mod convert;
mod gif;
mod jpeg;
pub(crate) mod limits;
mod png;
#[cfg(test)]
mod test_support;
pub(crate) mod tiff;
mod webp;

use std::io::Cursor;

use image::ImageResult;

use crate::pixels::{Rgb, release_over};
use pg_model::format::Format;

/// What the decoders keep from one file to the next, per worker (inside its [`crate::decode::Decoder`]). The format
/// crates' names are written `::png` and so on: inside this module, `png` is the [`png`] module.
#[derive(Default)]
pub(crate) struct Codecs {
    /// Samples that aren't 8-bit RGB or RGBA (gray, 16-bit, float, CMYK, planar TIFF), and a GIF's frame when it
    /// doesn't span the image's width: read here, then converted or composed into the output.
    raw: Vec<u8>,
    /// The PNG decoder's buffers.
    png: ::png::Scratch,
    /// The WebP decoder's buffers.
    webp: image_webp::Scratch,
    /// The GIF decoder's buffers.
    gif: ::gif::Scratch,
    /// The JPEG fallback's buffers (vendored zune-jpeg's `Scratch`).
    jpeg: zune_jpeg::Scratch,
    /// The TIFF decoder's decompressors and buffers (vendored tiff's `Scratch`).
    tiff: ::tiff::decoder::Scratch,
    /// A TIFF's sample formats.
    tiff_formats: Vec<u16>,
}

impl Codecs {
    /// Decode `bytes` (of `format`) into `out`, as 8-bit RGB with transparency flattened onto white.
    pub(crate) fn decode(&mut self, bytes: &[u8], format: Format, out: &mut Rgb) -> ImageResult<()> {
        let raw = &mut self.raw;
        match format {
            Format::Png => png::decode(&mut self.png, raw, bytes, out),
            Format::Gif => gif::decode(&mut self.gif, raw, bytes, out),
            Format::Webp => webp::decode(&mut self.webp, raw, bytes, out),
            Format::Bmp => convert::read_with(image::codecs::bmp::BmpDecoder::new(Cursor::new(bytes))?, raw, out),
            Format::Tiff => tiff::decode(&mut self.tiff, &mut self.tiff_formats, raw, bytes, out),
            Format::Jpeg => jpeg::decode(&mut self.jpeg, raw, bytes, out),
        }
    }

    /// Free what one oversize file grew.
    pub(crate) fn trim(&mut self, bytes: usize) {
        release_over(&mut self.raw, bytes);
        self.png.trim(bytes);
        self.webp.trim(bytes);
        self.gif.trim(bytes);
        self.jpeg.trim(bytes);
        self.tiff.trim(bytes);
    }
}

#[cfg(test)]
mod tests {
    use crate::fixtures::tiff_rgb8;
    use image::codecs::gif::GifEncoder;
    use image::{DynamicImage, ImageFormat, Rgba, RgbaImage};

    use super::*;
    use crate::codecs::test_support::{flattened, ne_bytes, noise};
    use pg_model::num;

    /// How the decode was done before this module: `image::load_from_memory_with_format`, then `into_rgba8` and
    /// the flattening onto white, or `into_rgb8`.
    fn reference(bytes: &[u8], format: ImageFormat) -> Result<(usize, usize, Vec<u8>), String> {
        let img = image::load_from_memory_with_format(bytes, format).map_err(|e| e.to_string())?;
        Ok(flattened(img))
    }

    fn decoded(codecs: &mut Codecs, bytes: &[u8], format: Format) -> Result<(usize, usize, Vec<u8>), String> {
        // A buffer left dirty by an earlier image, as a worker's is.
        let mut out = Rgb { width: 0, height: 0, pixels: vec![0xA5; 1000] };
        codecs.decode(bytes, format, &mut out).map_err(|e| e.to_string())?;
        assert_eq!(out.pixels.len(), out.width * out.height * 3);
        Ok((out.width, out.height, out.pixels))
    }

    /// A test pattern: gradients and noise, with an alpha ramp.
    fn pattern(w: u32, h: u32, seed: u32) -> RgbaImage {
        RgbaImage::from_fn(w, h, |x, y| {
            let n = (x.wrapping_mul(2654435761) ^ y.wrapping_mul(40503) ^ seed).wrapping_mul(2246822519) >> 24;
            Rgba([x * 255 / w, y * 255 / h, n, (x + y) * 255 / (w + h)].map(|v| u8::try_from(v).unwrap()))
        })
    }

    /// How a hand-made TIFF stores its samples.
    struct TiffKind {
        photometric: u16,
        bits: u16,
        count: u16,
        alpha: bool,
        float: bool,
        planar: bool,
    }

    /// A little-endian, uncompressed TIFF with one strip per plane. `samples` are the stored bytes, interleaved or
    /// one plane after another.
    fn tiff_file(w: u32, h: u32, kind: &TiffKind, samples: &[u8]) -> Vec<u8> {
        let count = usize::from(kind.count);
        let planes = if kind.planar { count } else { 1 };
        let strip = samples.len() / planes;
        let mut out = b"II*\0\0\0\0\0".to_vec();
        let offsets: Vec<u32> = (0..planes).map(|p| u32::try_from(out.len() + p * strip).unwrap()).collect();
        out.extend_from_slice(samples);
        // (tag, type, values): type 3 is SHORT, 4 LONG.
        let mut entries: Vec<(u16, u16, Vec<u32>)> = vec![
            (256, 4, vec![w]),
            (257, 4, vec![h]),
            (258, 3, vec![u32::from(kind.bits); count]),
            (259, 3, vec![1]),
            (262, 3, vec![u32::from(kind.photometric)]),
            (273, 4, offsets),
            (277, 3, vec![u32::from(kind.count)]),
            (278, 4, vec![h]),
            (279, 4, vec![u32::try_from(strip).unwrap(); planes]),
            (284, 3, vec![if kind.planar { 2 } else { 1 }]),
        ];
        if kind.alpha {
            entries.push((338, 3, vec![2]));
        }
        if kind.float {
            entries.push((339, 3, vec![3; count]));
        }
        // Values that don't fit an entry's 4 bytes go after the samples, then the directory.
        let (mut arrays, mut ifd) = (Vec::new(), Vec::new());
        let encoded: Vec<Vec<u8>> = entries
            .iter()
            .map(|(_, kind, values)| {
                values
                    .iter()
                    .flat_map(|&v| {
                        if *kind == 3 {
                            u16::try_from(v).unwrap().to_le_bytes().to_vec()
                        } else {
                            v.to_le_bytes().to_vec()
                        }
                    })
                    .collect()
            })
            .collect();
        let ifd_at = out.len() + encoded.iter().filter(|b| b.len() > 4).map(Vec::len).sum::<usize>();
        for ((tag, kind, values), mut bytes) in entries.iter().zip(encoded) {
            ifd.extend(tag.to_le_bytes());
            ifd.extend(kind.to_le_bytes());
            ifd.extend(u32::try_from(values.len()).unwrap().to_le_bytes());
            if bytes.len() <= 4 {
                bytes.resize(4, 0);
                ifd.extend(bytes);
            } else {
                ifd.extend(u32::try_from(out.len() + arrays.len()).unwrap().to_le_bytes());
                arrays.extend(bytes);
            }
        }
        out.extend(arrays);
        assert_eq!(out.len(), ifd_at);
        out[4..8].copy_from_slice(&u32::try_from(ifd_at).unwrap().to_le_bytes());
        out.extend(u16::try_from(entries.len()).unwrap().to_le_bytes());
        out.extend(ifd);
        out.extend(0u32.to_le_bytes());
        out
    }

    /// Every kind of file the encoders here (and hand-made TIFFs) can make: PNG of every colour type and depth,
    /// with and without tRNS, and APNG; GIF with frames that span the width and that don't; lossless WebP; BMP;
    /// TIFF gray, gray+alpha, RGB, RGBA and CMYK at their depths, interleaved and planar, floats and 1-bit gray;
    /// JPEG RGB, gray and CMYK through zune-jpeg.
    fn files() -> Vec<(String, Format, ImageFormat, Vec<u8>)> {
        let (w, h) = (37u32, 23u32);
        let mut files = Vec::new();
        for (ct, depths) in [
            (::png::ColorType::Grayscale, &[1u8, 2, 4, 8, 16][..]),
            (::png::ColorType::GrayscaleAlpha, &[8, 16]),
            (::png::ColorType::Rgb, &[8, 16]),
            (::png::ColorType::Rgba, &[8, 16]),
            (::png::ColorType::Indexed, &[1, 2, 4, 8]),
        ] {
            for &depth in depths {
                for trns in [false, true] {
                    if trns && matches!(ct, ::png::ColorType::GrayscaleAlpha | ::png::ColorType::Rgba) {
                        continue;
                    }
                    let samples = match ct {
                        ::png::ColorType::Grayscale | ::png::ColorType::Indexed => 1,
                        ::png::ColorType::GrayscaleAlpha => 2,
                        ::png::ColorType::Rgb => 3,
                        _ => 4,
                    };
                    let row_bits = num::index(w) * samples * usize::from(depth);
                    let data = noise(row_bits.div_ceil(8) * num::index(h), usize::from(depth));
                    let mut out = Vec::new();
                    {
                        let mut e = ::png::Encoder::new(&mut out, w, h);
                        e.set_color(ct);
                        e.set_depth(::png::BitDepth::from_u8(depth).unwrap());
                        if ct == ::png::ColorType::Indexed {
                            let n = 1usize << depth;
                            // The low byte of each product.
                            e.set_palette((0..n * 3).map(|i| (i * 37).to_le_bytes()[0]).collect::<Vec<_>>());
                            if trns {
                                e.set_trns((0..n).map(|i| (i * 91).to_le_bytes()[0]).collect::<Vec<_>>());
                            }
                        } else if trns {
                            let v = if depth == 16 { vec![0, 40] } else { vec![0, 40 & (u8::MAX >> (8 - depth))] };
                            e.set_trns(if ct == ::png::ColorType::Rgb { v.repeat(3) } else { v });
                        }
                        e.write_header().unwrap().write_image_data(&data).unwrap();
                    }
                    files.push((format!("png {ct:?} {depth} trns {trns}"), Format::Png, ImageFormat::Png, out));
                }
            }
        }
        {
            let mut out = Vec::new();
            let mut e = ::png::Encoder::new(&mut out, w, h);
            e.set_color(::png::ColorType::Rgba);
            e.set_animated(2, 0).unwrap();
            let mut wr = e.write_header().unwrap();
            for f in 0..2 {
                wr.write_image_data(pattern(w, h, f).as_raw()).unwrap();
            }
            drop(wr);
            files.push(("apng".into(), Format::Png, ImageFormat::Png, out));
        }
        // GIF frames on a w × h screen: the whole of it, part of it (composed), the full width with rows blank above
        // and below (read in place), past its edges, and an animation (only its first frame is read).
        for (name, frames) in [
            ("full", vec![(0, 0, w, h)]),
            ("offset", vec![(5, 3, w - 9, h - 7)]),
            ("full width", vec![(0, 4, w, h - 8)]),
            ("past the edges", vec![(30, 15, 20, 20)]),
            ("animated", vec![(0, 0, w, h), (3, 2, 10, 9)]),
        ] {
            let mut out = Vec::new();
            {
                let u16_of = |v: u32| u16::try_from(v).unwrap();
                let mut e = ::gif::Encoder::new(&mut out, u16_of(w), u16_of(h), &[]).unwrap();
                for (i, (left, top, fw, fh)) in frames.into_iter().enumerate() {
                    let mut rgba = pattern(fw, fh, u32::try_from(i).unwrap() + 1).into_raw();
                    let mut frame = ::gif::Frame::from_rgba_speed(u16_of(fw), u16_of(fh), &mut rgba, 10);
                    (frame.left, frame.top) = (u16_of(left), u16_of(top));
                    e.write_frame(&frame).unwrap();
                }
            }
            files.push((format!("gif {name}"), Format::Gif, ImageFormat::Gif, out));
        }
        // And through `image`'s encoder.
        let mut out = Vec::new();
        GifEncoder::new(&mut out).encode_frame(image::Frame::new(pattern(w, h, 4))).unwrap();
        files.push(("gif xmp icc".into(), Format::Gif, ImageFormat::Gif, with_app_extensions(&out, 3000)));
        files.push(("gif image".into(), Format::Gif, ImageFormat::Gif, out));
        let rgba = DynamicImage::ImageRgba8(pattern(w, h, 7));
        for (name, img) in [
            ("rgb8", DynamicImage::ImageRgb8(rgba.to_rgb8())),
            ("rgba8", rgba.clone()),
            ("l8", DynamicImage::ImageLuma8(rgba.to_luma8())),
            ("la8", DynamicImage::ImageLumaA8(rgba.to_luma_alpha8())),
        ] {
            for (format, image_format) in [(Format::Webp, ImageFormat::WebP), (Format::Bmp, ImageFormat::Bmp)] {
                let mut out = Cursor::new(Vec::new());
                if img.write_to(&mut out, image_format).is_ok() {
                    files.push((format!("{image_format:?} {name}"), format, image_format, out.into_inner()));
                }
            }
        }
        // TIFF: (photometric, bits, samples, alpha, float), interleaved and planar.
        let pixels = num::index(w * h);
        for (photometric, bits, count, alpha, float) in [
            (1, 8, 1, false, false),
            (1, 16, 1, false, false),
            (1, 8, 2, true, false),
            (1, 16, 2, true, false),
            (2, 8, 3, false, false),
            (2, 16, 3, false, false),
            (2, 8, 4, true, false),
            (2, 16, 4, true, false),
            (2, 32, 3, false, true),
            (2, 32, 4, true, true),
            (5, 8, 4, false, false),
            (5, 16, 4, false, false),
        ] {
            for planar in [false, true] {
                let len = pixels * usize::from(count) * usize::from(bits / 8);
                let samples = if float {
                    let floats: Vec<f32> = noise(len / 4, 3).iter().map(|&v| f32::from(v) / 200.0 - 0.1).collect();
                    ne_bytes(&floats, f32::to_le_bytes)
                } else {
                    noise(len, usize::from(bits))
                };
                let kind = TiffKind { photometric, bits, count, alpha, float, planar };
                let name = format!("tiff {photometric} {bits}x{count} planar {planar}");
                files.push((name, Format::Tiff, ImageFormat::Tiff, tiff_file(w, h, &kind, &samples)));
            }
        }
        let gray1 = TiffKind { photometric: 1, bits: 1, count: 1, alpha: false, float: false, planar: false };
        let samples = noise(num::index(w).div_ceil(8) * num::index(h), 1);
        files.push(("tiff gray 1".into(), Format::Tiff, ImageFormat::Tiff, tiff_file(w, h, &gray1, &samples)));
        // One RGB8 image, classic and BigTIFF, as the tiff crate's encoder writes them.
        let rgb = noise(pixels * 3, 8);
        files.push((
            "tiff rgb8 encoder".into(),
            Format::Tiff,
            ImageFormat::Tiff,
            tiff_rgb8(w, h, &rgb, false).unwrap(),
        ));
        files.push(("tiff rgb8 BigTIFF".into(), Format::Tiff, ImageFormat::Tiff, tiff_rgb8(w, h, &rgb, true).unwrap()));
        let cmyk: Vec<u8> = pattern(w, h, 9).as_raw().iter().map(|v| 255 - v).collect();
        for (name, format, pixels, sub) in [
            ("rgb", turbojpeg::PixelFormat::RGBA, pattern(w, h, 3).into_raw(), turbojpeg::Subsamp::Sub2x2),
            ("gray", turbojpeg::PixelFormat::GRAY, rgba.to_luma8().into_raw(), turbojpeg::Subsamp::Gray),
            ("cmyk", turbojpeg::PixelFormat::CMYK, cmyk, turbojpeg::Subsamp::None),
        ] {
            let pitch = num::index(w) * format.size();
            let image =
                turbojpeg::Image { pixels: &pixels[..], width: num::index(w), pitch, height: num::index(h), format };
            let jpeg = turbojpeg::compress(image, 90, sub).unwrap().to_vec();
            files.push((format!("jpeg {name}"), Format::Jpeg, ImageFormat::Jpeg, jpeg));
        }
        files
    }

    /// A GIF with XMP and ICC application extensions of `len` bytes each, placed before the first frame (after the
    /// header and the global palette).
    fn with_app_extensions(gif: &[u8], len: usize) -> Vec<u8> {
        let palette = if gif[10] & 0x80 != 0 { 3 << ((gif[10] & 7) + 1) } else { 0 };
        let at = 13 + palette;
        let mut out = gif[..at].to_vec();
        for (name, byte) in [(b"XMP DataXMP", b'x'), (b"ICCRGBG1012", b'c')] {
            out.extend([0x21, 0xFF, 11]);
            out.extend(name);
            for chunk in vec![byte; len].chunks(255) {
                out.push(u8::try_from(chunk.len()).unwrap());
                out.extend(chunk);
            }
            out.push(0);
        }
        out.extend(&gif[at..]);
        out
    }

    /// The one difference skipping a GIF's metadata makes: XMP over gif's 50 MB memory limit fails `image`'s decode,
    /// and this one decodes the pixels as `image` decodes the same file without it.
    #[test]
    fn gif_metadata_over_the_memory_limit_is_skipped() {
        let mut gif = Vec::new();
        GifEncoder::new(&mut gif).encode_frame(image::Frame::new(pattern(40, 30, 4))).unwrap();
        let huge = with_app_extensions(&gif, 51_000_000);
        assert!(reference(&huge, ImageFormat::Gif).is_err());
        let ours = decoded(&mut Codecs::default(), &huge, Format::Gif);
        assert_eq!(ours, reference(&gif, ImageFormat::Gif));
    }

    /// A PNG with one chunk inserted before its first IDAT, its CRC right: the damage is in the chunk's data.
    fn png_with_chunk(png: &[u8], kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let chunk = pg_container::png_chunk(kind, data).unwrap();
        let idat = png.windows(4).position(|w| w == b"IDAT").unwrap() - 4;
        [&png[..idat], &chunk, &png[idat..]].concat()
    }

    /// The text and ICC chunks are skipped, where `image` reads them. The damaged ones here (bad zlib, a truncated
    /// stream, an empty or over-long keyword or name, a missing terminator, a bad compression method or flag, text
    /// that isn't UTF-8) must decode as `image` decodes them: the same pixels, or the same error.
    #[test]
    fn png_damaged_text_and_icc_chunks_decode_as_image_does() {
        let mut file = Cursor::new(Vec::new());
        pattern(37, 23, 5).write_to(&mut file, ImageFormat::Png).unwrap();
        let png = file.into_inner();
        let zlib = |data: &[u8]| miniz_oxide::deflate::compress_to_vec_zlib(data, 6);
        let text = zlib(b"a text chunk's payload");
        let long = [b"a".repeat(80), b"\0\0".to_vec()].concat();
        let cases: Vec<(&str, &[u8; 4], Vec<u8>)> = vec![
            ("zTXt valid", b"zTXt", [b"Comment\0\0".to_vec(), text.clone()].concat()),
            ("zTXt bad zlib", b"zTXt", b"Comment\0\0not zlib".to_vec()),
            ("zTXt truncated", b"zTXt", [b"Comment\0\0".to_vec(), text[..text.len() / 2].to_vec()].concat()),
            ("zTXt empty keyword", b"zTXt", [b"\0\0".to_vec(), text.clone()].concat()),
            ("zTXt 80-byte keyword", b"zTXt", [long.clone(), text.clone()].concat()),
            ("zTXt no keyword end", b"zTXt", b"Comment".to_vec()),
            ("zTXt method 1", b"zTXt", [b"Comment\0\x01".to_vec(), text.clone()].concat()),
            ("iTXt valid", b"iTXt", [b"XML:com.adobe.xmp\0\0\0\0\0".to_vec(), b"<x/>".to_vec()].concat()),
            ("iTXt valid compressed", b"iTXt", [b"Comment\0\x01\0\0\0".to_vec(), text.clone()].concat()),
            ("iTXt bad zlib", b"iTXt", b"Comment\0\x01\0\0\0junk".to_vec()),
            ("iTXt flag 2", b"iTXt", b"Comment\0\x02\0\0\0x".to_vec()),
            ("iTXt empty keyword", b"iTXt", b"\0\0\0\0\0".to_vec()),
            ("iTXt truncated", b"iTXt", b"Comment\0".to_vec()),
            ("iTXt not UTF-8", b"iTXt", [b"Comment\0\0\0\0\0".to_vec(), vec![0xFF, 0xFE]].concat()),
            ("iCCP valid", b"iCCP", [b"icc\0\0".to_vec(), zlib(b"not a profile")].concat()),
            ("iCCP bad zlib", b"iCCP", b"icc\0\0junk".to_vec()),
            ("iCCP empty name", b"iCCP", [b"\0\0".to_vec(), zlib(b"x")].concat()),
            ("iCCP no name end", b"iCCP", b"icc".to_vec()),
            ("iCCP 80-byte name", b"iCCP", [long.clone(), zlib(b"x")].concat()),
        ];
        for (name, kind, data) in cases {
            let bytes = png_with_chunk(&png, kind, &data);
            let ours = decoded(&mut Codecs::default(), &bytes, Format::Png);
            if name.ends_with("valid") || name.ends_with("compressed") {
                assert!(ours.is_ok(), "{name}: {ours:?}");
            }
            assert_eq!(ours, reference(&bytes, ImageFormat::Png), "{name}");
        }
    }

    /// Every kind of file decodes to what `image` gave, or fails with its error, with one `Codecs` reused across
    /// them all, forwards and backwards, so that state left by one file would show in another.
    #[test]
    fn files_decode_as_image_did() {
        let files = files();
        let mut codecs = Codecs::default();
        let mut failed = Vec::new();
        for (name, format, image_format, bytes) in files.iter() {
            let result = decoded(&mut codecs, bytes, *format);
            assert_eq!(result, reference(bytes, *image_format), "{name}");
            if result.is_err() {
                failed.push(name.as_str());
            }
        }
        for (name, format, image_format, bytes) in files.iter().rev() {
            assert_eq!(decoded(&mut codecs, bytes, *format), reference(bytes, *image_format), "{name} (backwards)");
        }
        // Every kind made it. What fails fails as it did: gray with alpha, which the tiff crate reads as two bands
        // (`Multiband`, a colour type `image` doesn't take), and planar CMYK.
        assert_eq!(files.len(), 72);
        let tiff = |kind: &str| [false, true].map(|planar| format!("tiff {kind} planar {planar}"));
        let mut expected = [tiff("1 8x2"), tiff("1 16x2")].concat();
        expected.extend(["tiff 5 8x4 planar true".to_string(), "tiff 5 16x4 planar true".to_string()]);
        assert_eq!(failed, expected);
    }
}
