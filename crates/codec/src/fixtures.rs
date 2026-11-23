//! Test fixtures for the crates that test their files: the image files a test writes (PNG, BMP, GIF, TIFF, APNG, and
//! the encoders' JPEG and lossless WebP), a TIFF the tiff crate's encoder writes (BigTIFF, which `image` doesn't write),
//! and an image turned by an EXIF orientation, which the edit's and the library's tests compare against. Built with the
//! `fixture` feature, which the crates' dev-dependencies enable. The encoders are the ones the codec links, so the bytes
//! are the encoders' own; the fixtures' signatures name none of their types.

use pg_model::orientation::Orientation;

use crate::decode::transform;
use crate::encode::{Chroma, JpegEncoder};
use crate::error::{Error, Result};
use crate::pixels::Rgb;

/// Apply an EXIF orientation, giving the upright image.
pub fn orient(img: Rgb, orientation: Orientation) -> Rgb {
    let (pixels, width, height) = transform(&img.pixels, img.width, img.height, 3, orientation);
    Rgb { width, height, pixels }
}

/// A lossy WebP of `w` × `h` RGB pixels at `quality` (libwebp's 0–100), as the cut's re-encode writes one. Fails for
/// `pixels` that don't hold `w × h` RGB pixels.
pub fn webp_lossy_rgb(w: u32, h: u32, pixels: &[u8], quality: f32) -> Result<Vec<u8>> {
    let side = |n: u32| usize::try_from(n).map_err(|_| Error::invalid("image side over usize"));
    let expected = side(w)? * side(h)? * 3;
    if pixels.len() != expected {
        return Err(Error::invalid(format!("{} bytes for a {w}×{h} RGB image", pixels.len())));
    }
    Ok(webp::Encoder::from_rgb(pixels, w, h).encode(quality).to_vec())
}

/// An RGB8 TIFF written by the tiff crate's encoder, classic or BigTIFF (`image` writes only classic). Fails as the
/// encoder does, for one when `pixels` doesn't hold `w × h` RGB pixels.
pub fn tiff_rgb8(w: u32, h: u32, pixels: &[u8], big: bool) -> tiff::TiffResult<Vec<u8>> {
    use tiff::encoder::{TiffEncoder, colortype::RGB8};
    let mut out = std::io::Cursor::new(Vec::new());
    if big {
        TiffEncoder::new_big(&mut out)?.write_image::<RGB8>(w, h, pixels)?;
    } else {
        TiffEncoder::new(&mut out)?.write_image::<RGB8>(w, h, pixels)?;
    }
    Ok(out.into_inner())
}

/// `rgb` (`w` × `h` pixels, three bytes each) as a JPEG at `quality` (libjpeg-turbo's 1–100) in `chroma`'s
/// subsampling: the encoder the thumbnails use, so its bytes are the codec's own. Fails for `pixels` that don't hold
/// `width` × `height` RGB pixels.
pub fn jpeg_rgb(width: usize, height: usize, pixels: &[u8], quality: u8, chroma: Chroma) -> Result<Vec<u8>> {
    let img = Rgb { width, height, pixels: pixels.to_vec() };
    JpegEncoder::new(quality, chroma)?.encode(&img)
}

/// `rgb` (`w` × `h` pixels, three bytes each) as a lossless WebP, with `exif` as its EXIF chunk when it is given.
pub fn webp_lossless_rgb(w: u32, h: u32, rgb: &[u8], exif: Option<&[u8]>) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    {
        let mut encoder = image_webp::WebPEncoder::new(&mut out);
        if let Some(exif) = exif {
            encoder.set_exif_metadata(exif.to_vec());
        }
        encoder.encode(rgb, w, h, image_webp::ColorType::Rgb8).map_err(|e| Error::encode("encode the WebP", e))?;
    }
    Ok(out)
}

/// A PNG of `w` × `h` pixels with `channels` samples each (1 grey, 2 grey and alpha, 3 RGB, 4 RGBA) at `bits` per
/// sample (1, 2, 4, 8 or 16). Below 8 bits, `samples` holds one sample per byte, row after row, and is packed here
/// (MSB first, rows padded to whole bytes); at 8 or 16 bits it is the image data as PNG stores it (two bytes a sample,
/// big-endian, at 16).
pub fn png_layout(w: u32, h: u32, channels: u8, bits: u8, samples: &[u8]) -> Result<Vec<u8>> {
    let color = match channels {
        1 => png::ColorType::Grayscale,
        2 => png::ColorType::GrayscaleAlpha,
        3 => png::ColorType::Rgb,
        4 => png::ColorType::Rgba,
        _ => return Err(Error::invalid(format!("a PNG of {channels} channels"))),
    };
    let depth = match bits {
        1 => png::BitDepth::One,
        2 => png::BitDepth::Two,
        4 => png::BitDepth::Four,
        8 => png::BitDepth::Eight,
        16 => png::BitDepth::Sixteen,
        _ => return Err(Error::invalid(format!("a PNG of {bits} bits a sample"))),
    };
    let data = if bits < 8 { pack_samples(samples, w, h, channels, bits)? } else { samples.to_vec() };
    let mut file = Vec::new();
    let mut enc = png::Encoder::new(&mut file, w, h);
    enc.set_color(color);
    enc.set_depth(depth);
    let mut writer = enc.write_header().map_err(|e| Error::encode("write the PNG header", e))?;
    writer.write_image_data(&data).map_err(|e| Error::encode("write the PNG data", e))?;
    writer.finish().map_err(|e| Error::encode("finish the PNG", e))?;
    Ok(file)
}

/// `samples` (`channels` to a pixel, `w` pixels to a row, `h` rows; one per byte) packed `bits` to a sample, most
/// significant bit first, each row padded to a whole byte. `bits` is below 8: 1, 2 or 4.
fn pack_samples(samples: &[u8], w: u32, h: u32, channels: u8, bits: u8) -> Result<Vec<u8>> {
    let side = |n: u32| usize::try_from(n).map_err(|e| Error::invalid(e.to_string()));
    let (row, rows, bits) = (side(w)? * usize::from(channels), side(h)?, usize::from(bits));
    if samples.len() != row * rows {
        return Err(Error::invalid(format!("{} samples for a {w}×{h} PNG", samples.len())));
    }
    let stride = (row * bits).div_ceil(8);
    let mut out = vec![0u8; stride * rows];
    for (line, values) in out.chunks_exact_mut(stride.max(1)).zip(samples.chunks_exact(row.max(1))) {
        for (i, &value) in values.iter().enumerate() {
            if let Some(byte) = line.get_mut(i * bits / 8) {
                *byte |= value << (8 - bits - (i * bits) % 8);
            }
        }
    }
    Ok(out)
}

/// The encoders a test writes a picture with: the tests name none of the `image` crate's types, so this is theirs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    /// JPEG.
    Jpeg,
    /// PNG.
    Png,
    /// WebP.
    WebP,
    /// TIFF.
    Tiff,
    /// GIF.
    Gif,
    /// BMP.
    Bmp,
}

/// `rgb` (`w` × `h` pixels, three bytes each) written with `encoding`.
pub fn encode(w: u32, h: u32, rgb: &[u8], encoding: Encoding) -> Result<Vec<u8>> {
    let format = match encoding {
        Encoding::Jpeg => image::ImageFormat::Jpeg,
        Encoding::Png => image::ImageFormat::Png,
        Encoding::WebP => image::ImageFormat::WebP,
        Encoding::Tiff => image::ImageFormat::Tiff,
        Encoding::Gif => image::ImageFormat::Gif,
        Encoding::Bmp => image::ImageFormat::Bmp,
    };
    still(w, h, rgb, format)
}

/// `rgb` (`w` × `h` pixels, three bytes each) as a BMP.
pub fn bmp_rgb(w: u32, h: u32, rgb: &[u8]) -> Result<Vec<u8>> {
    still(w, h, rgb, image::ImageFormat::Bmp)
}

/// `rgb` as a GIF, one frame.
pub fn gif_rgb(w: u32, h: u32, rgb: &[u8]) -> Result<Vec<u8>> {
    still(w, h, rgb, image::ImageFormat::Gif)
}

/// `rgb` as a PNG.
pub fn png_rgb(w: u32, h: u32, rgb: &[u8]) -> Result<Vec<u8>> {
    still(w, h, rgb, image::ImageFormat::Png)
}

/// `rgb` as a classic TIFF (`image` writes no BigTIFF: [`tiff_rgb8`] does).
pub fn tiff_rgb(w: u32, h: u32, rgb: &[u8]) -> Result<Vec<u8>> {
    still(w, h, rgb, image::ImageFormat::Tiff)
}

/// `rgba` (`w` × `h` pixels, four bytes each) as a PNG, with its transparency.
pub fn png_rgba(w: u32, h: u32, rgba: &[u8]) -> Result<Vec<u8>> {
    let img = image::RgbaImage::from_raw(w, h, rgba.to_vec())
        .ok_or_else(|| Error::invalid(format!("{} bytes are not {w} × {h} RGBA pixels", rgba.len())))?;
    let mut file = std::io::Cursor::new(Vec::new());
    img.write_to(&mut file, image::ImageFormat::Png).map_err(|e| Error::encode("encode the PNG", e))?;
    Ok(file.into_inner())
}

/// An animated GIF of `frames` 4 × 4 frames, the first red and each after it brighter. The red level steps by 60, so
/// at most 5 frames (0 to 240): more is an error.
pub fn gif(frames: usize) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut enc = image::codecs::gif::GifEncoder::new(&mut out);
    for i in 0..frames {
        let red = i
            .checked_mul(60)
            .and_then(|level| u8::try_from(level).ok())
            .ok_or_else(|| Error::invalid(format!("frame {i} of {frames}: a test GIF has at most 5 frames")))?;
        let img = image::RgbaImage::from_pixel(4, 4, image::Rgba([red, 0, 0, 255]));
        enc.encode_frame(image::Frame::new(img)).map_err(|e| Error::encode(format!("encode GIF frame {i}"), e))?;
    }
    drop(enc);
    Ok(out)
}

/// An animated PNG of `frames` 2 × 2 frames of one grey, or a still PNG for one frame.
pub fn apng(frames: u32) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut enc = png::Encoder::new(&mut out, 2, 2);
    enc.set_color(png::ColorType::Rgb);
    if frames > 1 {
        enc.set_animated(frames, 0).map_err(|e| Error::encode("make the PNG animated", e))?;
    }
    let mut w = enc.write_header().map_err(|e| Error::encode("write the PNG header", e))?;
    for _ in 0..frames {
        w.write_image_data(&[7; 12]).map_err(|e| Error::encode("write a PNG frame", e))?;
    }
    w.finish().map_err(|e| Error::encode("finish the PNG", e))?;
    Ok(out)
}

/// `rgb` (`w` × `h` pixels, three bytes each) as an image of `format`, the one frame.
fn still(w: u32, h: u32, rgb: &[u8], format: image::ImageFormat) -> Result<Vec<u8>> {
    let img = image::RgbImage::from_raw(w, h, rgb.to_vec())
        .ok_or_else(|| Error::invalid(format!("{} bytes are not {w} × {h} RGB pixels", rgb.len())))?;
    let mut file = std::io::Cursor::new(Vec::new());
    img.write_to(&mut file, format).map_err(|e| Error::encode(format!("encode the {format:?}"), e))?;
    Ok(file.into_inner())
}
