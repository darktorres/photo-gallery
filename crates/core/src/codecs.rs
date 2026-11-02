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

use std::io::Cursor;

use image::error::{
    DecodingError, LimitError, LimitErrorKind, ParameterError, ParameterErrorKind, UnsupportedError,
    UnsupportedErrorKind,
};
use image::{ColorType, ExtendedColorType, ImageDecoder, ImageError, ImageFormat, ImageResult, Limits};

use crate::format::Format;

use crate::decode::{Rgb, release_over};
/// What the decoders keep from one file to the next, per worker (inside its [`crate::decode::Decoder`]).
#[derive(Default)]
pub(crate) struct Codecs {
    /// Samples that aren't 8-bit RGB or RGBA (gray, 16-bit, float, CMYK, planar TIFF), and a GIF's frame when it
    /// doesn't span the image's width: read here, then converted or composed into the output.
    raw: Vec<u8>,
    /// The PNG decoder's buffers.
    png: png::Scratch,
    /// The WebP decoder's buffers.
    webp: image_webp::Scratch,
    /// The GIF decoder's buffers.
    gif: gif::Scratch,
    /// The JPEG fallback's buffers (vendored zune-jpeg's `Scratch`).
    jpeg: zune_jpeg::Scratch,
    /// The TIFF decoder's decompressors and buffers (vendored tiff's `Scratch`).
    tiff: tiff::decoder::Scratch,
    /// A TIFF's sample formats.
    tiff_formats: Vec<u16>,
}

impl Codecs {
    /// Decode `bytes` (of `format`) into `out`, as 8-bit RGB with transparency flattened onto white.
    pub(crate) fn decode(&mut self, bytes: &[u8], format: Format, out: &mut Rgb) -> ImageResult<()> {
        match format {
            Format::Png => self.png(bytes, out),
            Format::Gif => self.gif(bytes, out),
            Format::Webp => self.webp(bytes, out),
            Format::Bmp => self.read_with(image::codecs::bmp::BmpDecoder::new(Cursor::new(bytes))?, out),
            Format::Tiff => self.tiff(bytes, out),
            Format::Jpeg => self.jpeg(bytes, out),
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

    /// Any of `image`'s decoders, the way `ImageReader::decode` and `decoder_to_image` use it (BMP's: `image` is
    /// its format crate).
    fn read_with(&mut self, mut decoder: impl ImageDecoder, out: &mut Rgb) -> ImageResult<()> {
        let ((w, h), color) = (decoder.dimensions(), decoder.color_type());
        let mut limits = Limits::default();
        limits.reserve(decoder.total_bytes())?;
        decoder.set_limits(limits)?;
        decoder.read_image(target(color, w, h, out, &mut self.raw))?;
        finish(color, w, h, out, &self.raw)
    }

    /// `image`'s `PngDecoder::with_limits` and `read_image`, in the buffers of the PNG before (vendored png's
    /// `Scratch`), which go back to `self.png` whatever happens.
    fn png(&mut self, bytes: &[u8], out: &mut Rgb) -> ImageResult<()> {
        // The PNG decoder is given `ImageReader`'s limits as they are before the image's size is reserved.
        let max_bytes = usize::try_from(Limits::default().max_alloc.unwrap_or(u64::MAX)).unwrap_or(usize::MAX);
        let limits = png::Limits { bytes: max_bytes };
        let mut decoder = png::Decoder::new_with_scratch(Cursor::new(bytes), limits, std::mem::take(&mut self.png));
        // Text and ICC profiles are skipped, where `image` reads them for its metadata getters, which a decode
        // doesn't call: the metadata reader finds the XMP itself, and colours aren't managed. The png crate
        // ignores what is wrong in them, so skipping them changes no pixel and no error, except for a file whose
        // text alone exceeds the 512 MiB limit; it spares a copy of each.
        decoder.set_ignore_text_chunk(true);
        decoder.set_ignore_iccp_chunk(true);
        if let Err(err) = decoder.read_header_info() {
            self.png = decoder.into_scratch();
            return Err(png_error(err));
        }
        // 16-bit samples kept, samples of fewer bits expanded to 8, palettes to RGB(A), tRNS to alpha.
        decoder.set_transformations(png::Transformations::EXPAND);
        let mut reader = match decoder.read_info_or_scratch() {
            Ok(reader) => reader,
            Err((err, scratch)) => {
                self.png = scratch;
                return Err(png_error(err));
            }
        };
        let result = self.png_image(&mut reader, out);
        self.png = reader.into_scratch();
        result
    }

    /// [`Codecs::png`]'s image, once the headers are read.
    fn png_image(&mut self, reader: &mut png::Reader<Cursor<&[u8]>>, out: &mut Rgb) -> ImageResult<()> {
        let color = png_color(reader.output_color_type())?;
        let (w, h) = reader.info().size();
        reserve(color, w, h)?;
        let buf = target(color, w, h, out, &mut self.raw);
        reader.next_frame(buf).map_err(png_error)?;
        // PNG's samples are big-endian; `read_image` gives them in native order.
        if color.bytes_per_pixel() / color.channel_count() == 2 {
            for c in buf.as_chunks_mut::<2>().0 {
                *c = u16::from_be_bytes(*c).to_ne_bytes();
            }
        }
        finish(color, w, h, out, &self.raw)
    }

    /// `image`'s `GifDecoder::new` and `read_image`: the first frame, as RGBA. In the buffers of the GIF before
    /// (vendored gif's `Scratch`), which go back to `self.gif` whatever happens, and read straight from `bytes`, where
    /// `image` reads them through a `BufReader`.
    fn gif(&mut self, bytes: &[u8], out: &mut Rgb) -> ImageResult<()> {
        let mut options = gif::DecodeOptions::new();
        options.set_color_output(gif::ColorOutput::RGBA);
        // The XMP and ICC extensions are skipped, where `image`'s decoder collects them for its metadata getters,
        // which a decode doesn't call: the metadata reader finds the XMP itself, and colours aren't managed. That
        // changes no pixel and no error, except for a file whose XMP or ICC alone is over gif's 50 MB memory limit,
        // which `image` fails and this decodes.
        options.skip_metadata(true);
        let mut reader = match options.read_info_with(Cursor::new(bytes), std::mem::take(&mut self.gif)) {
            Ok(reader) => reader,
            Err((err, scratch)) => {
                self.gif = scratch;
                return Err(gif_error(err));
            }
        };
        let result = self.gif_image(&mut reader, out);
        self.gif = reader.into_scratch();
        result
    }

    /// [`Codecs::gif`]'s image, once the headers are read.
    fn gif_image(&mut self, reader: &mut gif::Decoder<Cursor<&[u8]>, Cursor<&[u8]>>, out: &mut Rgb) -> ImageResult<()> {
        let (w, h) = (u32::from(reader.width()), u32::from(reader.height()));
        let mut limits = reserve(ColorType::Rgba8, w, h)?;
        let buf = zeroed(&mut out.pixels, w as usize * h as usize * 4);
        let Some(frame) = reader.next_frame_info().map_err(gif_error)? else {
            return Err(ImageError::Parameter(ParameterError::from_kind(ParameterErrorKind::NoMoreData)));
        };
        let [left, top, fw, fh] = [frame.left, frame.top, frame.width, frame.height].map(u32::from);
        if left == 0 && fw == w && u64::from(top) + u64::from(fh) <= u64::from(h) {
            // A frame that spans the image's width is read in place; the rows above and below it stay zero.
            let line = w as usize * 4;
            let rows = &mut buf[line * top as usize..][..line * fh as usize];
            reader.read_into_buffer(rows).map_err(gif_error)?;
        } else {
            // Any other frame is read on its own, then placed: what it doesn't cover stays zero (transparent).
            let size = (fw as usize)
                .checked_mul(fh as usize)
                .and_then(|s| s.checked_mul(4))
                .ok_or(ImageError::Limits(LimitError::from_kind(LimitErrorKind::InsufficientMemory)))?;
            limits.reserve_usize(size)?;
            let frame = zeroed(&mut self.raw, size);
            limits.free_usize(size);
            reader.read_into_buffer(frame).map_err(gif_error)?;
            for y in 0..h {
                let fy = y.wrapping_sub(top);
                if fy >= fh {
                    continue;
                }
                for x in 0..w {
                    let fx = x.wrapping_sub(left);
                    if fx < fw {
                        let (s, d) = (((fy * fw + fx) * 4) as usize, ((y * w + x) * 4) as usize);
                        buf[d..d + 4].copy_from_slice(&frame[s..s + 4]);
                    }
                }
            }
        }
        finish(ColorType::Rgba8, w, h, out, &self.raw)
    }

    /// `image`'s `WebPDecoder`, in the buffers of the WebP before (vendored image-webp's `Scratch`), which go back
    /// to `self.webp` whatever happens.
    fn webp(&mut self, bytes: &[u8], out: &mut Rgb) -> ImageResult<()> {
        let mut decoder =
            match image_webp::WebPDecoder::new_with_scratch(Cursor::new(bytes), std::mem::take(&mut self.webp)) {
                Ok(decoder) => decoder,
                Err((err, scratch)) => {
                    self.webp = scratch;
                    return Err(webp_error(err));
                }
            };
        let result = self.webp_image(&mut decoder, out);
        self.webp = decoder.into_scratch();
        result
    }

    /// [`Codecs::webp`]'s image, once the headers are read: RGBA if it has alpha, RGB otherwise.
    fn webp_image(&mut self, decoder: &mut image_webp::WebPDecoder<Cursor<&[u8]>>, out: &mut Rgb) -> ImageResult<()> {
        let (w, h) = decoder.dimensions();
        let color = if decoder.has_alpha() { ColorType::Rgba8 } else { ColorType::Rgb8 };
        reserve(color, w, h)?;
        decoder.read_image(target(color, w, h, out, &mut self.raw)).map_err(webp_error)?;
        finish(color, w, h, out, &self.raw)
    }

    /// `image`'s `JpegDecoder` (zune-jpeg), for the JPEGs libjpeg-turbo rejects. As there, one decoder reads the
    /// headers and another, set up from them, decodes: the colour conversion zune-jpeg uses is chosen as it reads
    /// the headers, from the output colour space it was given. Both decode in the buffers of the JPEG before
    /// (vendored zune-jpeg's `Scratch`), which go back to `self.jpeg` whatever happens.
    ///
    /// The metadata segments (EXIF, XMP, ICC, IPTC…) are checked as `image`'s decoder checks them, with the same
    /// errors, but not copied (`set_skip_metadata`): `image` keeps them for its metadata getters, which a decode
    /// doesn't call.
    fn jpeg(&mut self, bytes: &[u8], out: &mut Rgb) -> ImageResult<()> {
        use zune_core::bytestream::ZCursor;
        use zune_core::colorspace::ColorSpace;
        use zune_core::options::DecoderOptions;
        use zune_jpeg::JpegDecoder;

        let options =
            || DecoderOptions::default().set_strict_mode(false).set_max_width(usize::MAX).set_max_height(usize::MAX);
        let mut headers = JpegDecoder::new_with_scratch(ZCursor::new(bytes), options(), std::mem::take(&mut self.jpeg));
        headers.set_skip_metadata(true);
        let read = headers.decode_headers();
        if let Err(e) = read {
            self.jpeg = headers.into_scratch();
            return Err(jpeg_error(e));
        }
        // Known once the headers are read; a JPEG's sides fit 16 bits.
        let (w, h) = headers.dimensions().expect("headers were decoded");
        let (w, h) = (u32::from(u16::try_from(w).unwrap()), u32::from(u16::try_from(h).unwrap()));
        // Gray and RGB(A) are kept; the rest (CMYK, YCCK…) is converted to RGB by the decoder.
        let (space, color) = match headers.input_colorspace().expect("headers were decoded") {
            ColorSpace::RGB => (ColorSpace::RGB, ColorType::Rgb8),
            ColorSpace::RGBA => (ColorSpace::RGBA, ColorType::Rgba8),
            ColorSpace::Luma => (ColorSpace::Luma, ColorType::L8),
            ColorSpace::LumaA => (ColorSpace::LumaA, ColorType::La8),
            _ => (ColorSpace::RGB, ColorType::Rgb8),
        };
        let scratch = headers.into_scratch();
        if let Err(e) = reserve(color, w, h) {
            self.jpeg = scratch;
            return Err(e);
        }
        let mut decoder =
            JpegDecoder::new_with_scratch(ZCursor::new(bytes), options().jpeg_set_out_colorspace(space), scratch);
        decoder.set_skip_metadata(true);
        let decoded = decoder.decode_into(target(color, w, h, out, &mut self.raw)).map_err(jpeg_error);
        self.jpeg = decoder.into_scratch();
        decoded?;
        finish(color, w, h, out, &self.raw)
    }

    /// `image`'s `TiffDecoder`: its colour types, its split of the limits between the decoded image and the
    /// decoder's own buffers, and its conversions (CMYK to RGB, 1-bit gray to 8, planes interleaved), here done as
    /// the samples are converted into `out` rather than into a second full-size buffer first.
    fn tiff(&mut self, bytes: &[u8], out: &mut Rgb) -> ImageResult<()> {
        let decoder = match tiff::decoder::Decoder::new_with_scratch(Cursor::new(bytes), std::mem::take(&mut self.tiff))
        {
            Ok(decoder) => decoder,
            Err((e, scratch)) => {
                self.tiff = scratch;
                return Err(tiff_error(e));
            }
        };
        let mut slot = Some(decoder);
        let decoded = self.tiff_from(&mut slot, out);
        self.tiff = slot.expect("the decoder is back in its slot").into_scratch();
        decoded
    }

    /// [`Codecs::tiff`] from the decoder in `slot`, which `with_limits` takes out and puts back.
    fn tiff_from(&mut self, slot: &mut Option<TiffDecoder<'_>>, out: &mut Rgb) -> ImageResult<()> {
        use tiff::ColorType as Tiff;
        use tiff::decoder::DecodingSampleType as Sample;

        let decoder = slot.as_mut().expect("the decoder is in its slot");
        let (w, h) = decoder.dimensions().map_err(tiff_error)?;
        let tiff_color = decoder.colortype().map_err(tiff_error)?;
        // `find_tag_unsigned_vec`, into a kept vector; without the tag, unsigned integers.
        let formats = &mut self.tiff_formats;
        if decoder.find_tag_unsigned_vec_into(tiff::tags::Tag::SampleFormat, formats).map_err(tiff_error)? {
            formats.iter().try_for_each(|&f| check_sample_format(f, tiff_color))?;
        }
        let (color, stored) = match tiff_color {
            Tiff::Gray(1) => (ColorType::L8, Stored::Gray1),
            Tiff::Gray(8) => (ColorType::L8, Stored::Same),
            Tiff::Gray(16) => (ColorType::L16, Stored::Same),
            Tiff::GrayA(8) => (ColorType::La8, Stored::Same),
            Tiff::GrayA(16) => (ColorType::La16, Stored::Same),
            Tiff::RGB(8) => (ColorType::Rgb8, Stored::Same),
            Tiff::RGB(16) => (ColorType::Rgb16, Stored::Same),
            Tiff::RGBA(8) => (ColorType::Rgba8, Stored::Same),
            Tiff::RGBA(16) => (ColorType::Rgba16, Stored::Same),
            Tiff::CMYK(8) => (ColorType::Rgb8, Stored::Cmyk8),
            Tiff::CMYK(16) => (ColorType::Rgb16, Stored::Cmyk16),
            Tiff::RGB(32) => (ColorType::Rgb32F, Stored::Same),
            Tiff::RGBA(32) => (ColorType::Rgba32F, Stored::Same),
            Tiff::Palette(n) | Tiff::Gray(n) => return Err(tiff_unknown_color(n)),
            Tiff::GrayA(n) => return Err(tiff_unknown_color(n.saturating_mul(2))),
            Tiff::RGB(n) | Tiff::YCbCr(n) => return Err(tiff_unknown_color(n.saturating_mul(3))),
            Tiff::RGBA(n) | Tiff::CMYK(n) => return Err(tiff_unknown_color(n.saturating_mul(4))),
            Tiff::Multiband { bit_depth, num_samples } => {
                return Err(tiff_unknown_color(bit_depth.saturating_mul(num_samples.min(255) as u8)));
            }
            _ => return Err(tiff_unknown_color(0)),
        };
        let limits = reserve(color, w, h)?;

        // `TiffDecoder::set_limits`: what the stored samples take (CMYK's 4 or 8 bytes a pixel, more than the RGB
        // image) for the decoding buffer, the rest of the allowance for the decoder's intermediate buffers.
        let max_alloc = limits.max_alloc.unwrap_or(u64::MAX);
        let stored_bpp = match stored {
            Stored::Cmyk8 => 4,
            Stored::Cmyk16 => 8,
            _ => u64::from(color.bytes_per_pixel()),
        };
        let stored_bytes = (u64::from(w) * u64::from(h)).saturating_mul(stored_bpp);
        let intermediate = max_alloc.saturating_sub(stored_bytes);
        let mut tiff_limits = tiff::decoder::Limits::default();
        tiff_limits.decoding_buffer_size = usize::try_from(max_alloc - intermediate).unwrap_or(usize::MAX);
        tiff_limits.intermediate_buffer_size = usize::try_from(intermediate).unwrap_or(usize::MAX);
        tiff_limits.ifd_value_size = tiff_limits.intermediate_buffer_size;
        let decoding_size = tiff_limits.decoding_buffer_size;
        let limited = slot.take().expect("the decoder is in its slot").with_limits(tiff_limits);
        let decoder = slot.insert(limited);

        // `Decoder::read_image_to_buffer`: all planes if they fit the decoding limit, else the first (and `image`
        // then fails); the buffer in whole samples, within the limit.
        let layout = decoder.image_buffer_layout().map_err(tiff_error)?;
        let sample_size = match layout.sample_type {
            Some(Sample::U8 | Sample::I8) => 1,
            Some(Sample::U16 | Sample::I16 | Sample::F16) => 2,
            Some(Sample::U32 | Sample::I32 | Sample::F32) => 4,
            Some(_) => 8,
            // Sample formats and depths that `check_sample_format` and the colour types above let through all
            // have a sample type.
            None => return Err(tiff_error(tiff::TiffError::LimitsExceeded)),
        };
        let plane_stride = layout.plane_stride.map_or(0, |n| n.get());
        let extent =
            if layout.complete_len <= decoding_size || layout.planes == 1 { layout.complete_len } else { plane_stride };
        let samples = extent.div_ceil(sample_size);
        if samples > decoding_size / sample_size {
            return Err(tiff_error(tiff::TiffError::LimitsExceeded));
        }
        let len = samples * sample_size;
        // Stored as the image is: read straight into `out`.
        let direct = layout.planes == 1
            && stored == Stored::Same
            && matches!(color, ColorType::Rgb8 | ColorType::Rgba8)
            && len == w as usize * h as usize * usize::from(color.bytes_per_pixel());
        let buf = if direct { zeroed(&mut out.pixels, len) } else { zeroed(&mut self.raw, len) };
        decoder.read_image_bytes(buf).map_err(tiff_error)?;
        if len < layout.complete_len {
            return Err(ImageError::Limits(LimitError::from_kind(LimitErrorKind::InsufficientMemory)));
        }
        if direct {
            return finish(color, w, h, out, &self.raw);
        }

        let pixels = w as usize * h as usize;
        if layout.planes > 1 {
            // `TiffDecoder::interleave_planes`.
            if stored != Stored::Same {
                return Err(ImageError::Unsupported(UnsupportedError::from_format_and_kind(
                    ImageFormat::Tiff.into(),
                    UnsupportedErrorKind::GenericFeature(
                        "Planar TIFF with CMYK color type is not supported".to_string(),
                    ),
                )));
            }
            // (An empty image has no planes to read: `image` panics there, at its tiff.rs:163, `chunks_exact(0)`.)
            if self.raw.len().checked_div(plane_stride).unwrap_or(0) < usize::from(color.channel_count()) {
                return Err(ImageError::Decoding(DecodingError::new(
                    ImageFormat::Tiff.into(),
                    "Not enough planes read from TIFF image".to_string(),
                )));
            }
            convert(&self.raw, color, Some(plane_stride), pixels, out)?;
            (out.width, out.height) = (w as usize, h as usize);
            return Ok(());
        }
        match stored {
            Stored::Same => convert(&self.raw, color, None, pixels, out)?,
            Stored::Cmyk8 => {
                // `image` writes each pixel with `write_all` (its tiff.rs:396): short data leaves the rest of the
                // pixels as they were, with no error.
                check_len(self.raw.len(), pixels * 4)?;
                let px = zeroed(&mut out.pixels, pixels * 3);
                for (cmyk, rgb) in self.raw.as_chunks::<4>().0.iter().zip(px.as_chunks_mut::<3>().0) {
                    *rgb = cmyk_to_rgb(cmyk);
                }
            }
            Stored::Cmyk16 => {
                // As Cmyk8: `image`'s tiff.rs:402 `write_all` leaves the rest of the pixels as they were.
                check_len(self.raw.len(), pixels * 8)?;
                let px = zeroed(&mut out.pixels, pixels * 3);
                for (cmyk, rgb) in self.raw.as_chunks::<8>().0.iter().zip(px.as_chunks_mut::<3>().0) {
                    let cmyk = [0, 1, 2, 3].map(|i| u16::from_ne_bytes([cmyk[2 * i], cmyk[2 * i + 1]]));
                    *rgb = cmyk_to_rgb16(&cmyk).map(rgb16);
                }
            }
            Stored::Gray1 => {
                // `expand_bits(1, …)` per row (0 or 255), then L8 to RGB.
                let row_bytes = (w as usize).div_ceil(8);
                // `image`'s tiff.rs:410 `chunks_exact` drops a short last row: its rows past the data are left unwritten.
                check_len(self.raw.len(), row_bytes * h as usize)?;
                let px = zeroed(&mut out.pixels, pixels * 3);
                for (row, out_row) in self.raw.chunks_exact(row_bytes.max(1)).zip(px.chunks_exact_mut(w as usize * 3)) {
                    for (x, rgb) in out_row.as_chunks_mut::<3>().0.iter_mut().enumerate() {
                        let bit = row[x / 8] >> (7 - x % 8) & 1;
                        *rgb = [luma8([bit * 255]); 3];
                    }
                }
            }
        }
        (out.width, out.height) = (w as usize, h as usize);
        Ok(())
    }
}

/// How a TIFF's samples are stored, against the colour type `image` gives the image.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Stored {
    /// As the colour type.
    Same,
    /// 1-bit gray, given as 8-bit (`L8`).
    Gray1,
    /// 8-bit CMYK, given as `Rgb8`.
    Cmyk8,
    /// 16-bit CMYK, given as `Rgb16`.
    Cmyk16,
}

/// `ImageReader::decode`'s reservation of the decoded image's size (`total_bytes`) from the default limits: an
/// image over 512 MiB fails here. The limits left are what the decoder is given.
fn reserve(color: ColorType, w: u32, h: u32) -> ImageResult<Limits> {
    let mut limits = Limits::default();
    let total = (u64::from(w) * u64::from(h)).saturating_mul(u64::from(color.bytes_per_pixel()));
    limits.reserve(total)?;
    Ok(limits)
}

/// The buffer a decoder writes a `color` image into: `out`'s own when it is 8-bit RGB or RGBA, else `raw`. Zeroed,
/// as `decoder_to_vec`'s new buffer is, so that what a decoder leaves unwritten is zero whatever the buffer held.
fn target<'a>(color: ColorType, w: u32, h: u32, out: &'a mut Rgb, raw: &'a mut Vec<u8>) -> &'a mut [u8] {
    let len = w as usize * h as usize * usize::from(color.bytes_per_pixel());
    match color {
        ColorType::Rgb8 | ColorType::Rgba8 => zeroed(&mut out.pixels, len),
        _ => zeroed(raw, len),
    }
}

/// `buf` resized to `len` bytes, all zero.
fn zeroed(buf: &mut Vec<u8>, len: usize) -> &mut [u8] {
    buf.clear();
    buf.resize(len, 0);
    buf
}

/// The decoded image into 8-bit RGB in `out`: as it is, flattened in place, or converted from `raw`.
fn finish(color: ColorType, w: u32, h: u32, out: &mut Rgb, raw: &[u8]) -> ImageResult<()> {
    let pixels = w as usize * h as usize;
    match color {
        ColorType::Rgb8 => {}
        ColorType::Rgba8 => {
            // Pixel i's RGB is written at 3i, which is below its RGBA at 4i, so nothing not yet read is overwritten.
            let px = &mut out.pixels;
            for i in 0..pixels {
                let a = px[4 * i + 3];
                for c in 0..3 {
                    px[3 * i + c] = over_white(px[4 * i + c], a);
                }
            }
            px.truncate(pixels * 3);
        }
        _ => convert(raw, color, None, pixels, out)?,
    }
    (out.width, out.height) = (w as usize, h as usize);
    Ok(())
}

/// `pixels` pixels of `color` (native-endian samples, interleaved, or in planes `plane_stride` bytes apart) into
/// 8-bit RGB in `out.pixels`, as `image` converts them (`into_rgb8`, or `into_rgba8` then flattened onto white when
/// the colour type has alpha).
fn convert(src: &[u8], color: ColorType, plane_stride: Option<usize>, pixels: usize, out: &mut Rgb) -> ImageResult<()> {
    let px = &mut out.pixels;
    match color {
        ColorType::L8 => to_rgb::<1, 1>(src, plane_stride, pixels, px, luma8),
        ColorType::La8 => to_rgb::<2, 1>(src, plane_stride, pixels, px, luma8),
        ColorType::Rgb8 => to_rgb::<3, 1>(src, plane_stride, pixels, px, |[v]| v),
        ColorType::Rgba8 => to_rgb::<4, 1>(src, plane_stride, pixels, px, |[v]| v),
        ColorType::L16 => to_rgb::<1, 2>(src, plane_stride, pixels, px, |s| luma16(u16::from_ne_bytes(s))),
        ColorType::La16 => to_rgb::<2, 2>(src, plane_stride, pixels, px, |s| luma16(u16::from_ne_bytes(s))),
        ColorType::Rgb16 => to_rgb::<3, 2>(src, plane_stride, pixels, px, |s| rgb16(u16::from_ne_bytes(s))),
        ColorType::Rgba16 => to_rgb::<4, 2>(src, plane_stride, pixels, px, |s| rgb16(u16::from_ne_bytes(s))),
        ColorType::Rgb32F => to_rgb::<3, 4>(src, plane_stride, pixels, px, |s| rgb_f32(f32::from_ne_bytes(s))),
        ColorType::Rgba32F => to_rgb::<4, 4>(src, plane_stride, pixels, px, |s| rgb_f32(f32::from_ne_bytes(s))),
        _ => Err(ImageError::Unsupported(UnsupportedError::from_format_and_kind(
            image::error::ImageFormatHint::Unknown,
            UnsupportedErrorKind::Color(color.into()),
        ))),
    }
}

/// [`convert`] for `CH` channels of `N`-byte samples, each made 8-bit by `to_u8`. One or two channels are gray
/// (and alpha), three or four RGB (and alpha); the last of two or four is alpha.
fn to_rgb<const CH: usize, const N: usize>(
    src: &[u8],
    plane_stride: Option<usize>,
    pixels: usize,
    px: &mut Vec<u8>,
    to_u8: impl Fn([u8; N]) -> u8,
) -> ImageResult<()> {
    match plane_stride {
        None => check_len(src.len(), pixels * CH * N)?,
        // The caller checked there are `CH` planes.
        Some(stride) => check_len(stride, pixels * N)?,
    }
    let sample = |i: usize, c: usize| {
        let at = match plane_stride {
            None => (i * CH + c) * N,
            Some(stride) => c * stride + i * N,
        };
        to_u8(src[at..at + N].try_into().unwrap())
    };
    let out = zeroed(px, pixels * 3);
    for (i, rgb) in out.as_chunks_mut::<3>().0.iter_mut().enumerate() {
        let v = if CH >= 3 { [sample(i, 0), sample(i, 1), sample(i, 2)] } else { [sample(i, 0); 3] };
        *rgb = if CH.is_multiple_of(2) {
            let a = sample(i, CH - 1);
            v.map(|v| over_white(v, a))
        } else {
            v
        };
    }
    Ok(())
}

/// The decoders give exactly the size the image takes: anything else is a decoder's bug. `image` would panic where
/// the lengths differ at its `copy_from_slice` (tiff.rs:417–444), or leave pixels unwritten (tiff.rs:396, 402, 410).
/// The sites stay errors: a panic would end a worker.
fn check_len(len: usize, expected: usize) -> ImageResult<()> {
    if len >= expected {
        return Ok(());
    }
    Err(ImageError::Decoding(DecodingError::new(
        image::error::ImageFormatHint::Unknown,
        format!("decoded {len} bytes, the image takes {expected}"),
    )))
}

/// A channel flattened onto white by its alpha, as the evaluation did (`eval/common.py::load_rgb`).
fn over_white(v: u8, a: u8) -> u8 {
    let (v, a) = (u32::from(v), u32::from(a));
    ((v * a + 255 * (255 - a) + 127) / 255) as u8
}

/// An 8-bit gray sample into RGB: `image` converts gray to RGB through `f32` (`cast_pixels_by_fallback`:
/// `expand_to_f32`, then `clamp_from_f32`).
fn luma8([v]: [u8; 1]) -> u8 {
    const R: f32 = 1.0 / u8::MAX as f32;
    clamp_u8(f32::from(v) * R)
}

/// [`luma8`] for 16-bit samples.
fn luma16(v: u16) -> u8 {
    const R: f32 = 1.0 / u16::MAX as f32;
    clamp_u8(f32::from(v) * R)
}

/// `clamp_from_f32` for `u8`: the cast saturates.
fn clamp_u8(v: f32) -> u8 {
    (v * u8::MAX as f32).round() as u8
}

/// A 16-bit RGB sample into 8 bits (`FromPrimitive<u16> for u8`): `round(v × 255 / 65535)`.
fn rgb16(v: u16) -> u8 {
    ((u32::from(v) + 128) / 257) as u8
}

/// A float RGB sample into 8 bits (`FromPrimitive<f32> for u8`, `normalize_float`): clamped to [0, 1], NaN to 1.
#[allow(clippy::neg_cmp_op_on_partial_ord)]
fn rgb_f32(v: f32) -> u8 {
    let clamped = if !(v < 1.0) { 1.0 } else { v.max(0.0) };
    (clamped * u8::MAX as f32).round() as u8
}

/// `image`'s `cmyk_to_rgb` (TIFF).
fn cmyk_to_rgb(cmyk: &[u8; 4]) -> [u8; 3] {
    let [c, m, y, k] = cmyk.map(f32::from);
    let kf = 1. - k / 255.;
    [((255. - c) * kf) as u8, ((255. - m) * kf) as u8, ((255. - y) * kf) as u8]
}

/// `image`'s `cmyk_to_rgb16` (TIFF).
fn cmyk_to_rgb16(cmyk: &[u16; 4]) -> [u16; 3] {
    let [c, m, y, k] = cmyk.map(f32::from);
    let kf = 1. - k / 65535.;
    [((65535. - c) * kf) as u16, ((65535. - m) * kf) as u16, ((65535. - y) * kf) as u16]
}

/// `image`'s colour type for a PNG's output (`PngDecoder::with_limits`).
fn png_color((color, bits): (png::ColorType, png::BitDepth)) -> ImageResult<ColorType> {
    use png::BitDepth as B;
    use png::ColorType as C;
    let unsupported = match (color, bits) {
        (C::Grayscale, B::Eight) => return Ok(ColorType::L8),
        (C::Grayscale, B::Sixteen) => return Ok(ColorType::L16),
        (C::GrayscaleAlpha, B::Eight) => return Ok(ColorType::La8),
        (C::GrayscaleAlpha, B::Sixteen) => return Ok(ColorType::La16),
        (C::Rgb, B::Eight) => return Ok(ColorType::Rgb8),
        (C::Rgb, B::Sixteen) => return Ok(ColorType::Rgb16),
        (C::Rgba, B::Eight) => return Ok(ColorType::Rgba8),
        (C::Rgba, B::Sixteen) => return Ok(ColorType::Rgba16),
        (C::Grayscale, B::One) => ExtendedColorType::L1,
        (C::GrayscaleAlpha, B::One) => ExtendedColorType::La1,
        (C::Rgb, B::One) => ExtendedColorType::Rgb1,
        (C::Rgba, B::One) => ExtendedColorType::Rgba1,
        (C::Grayscale, B::Two) => ExtendedColorType::L2,
        (C::GrayscaleAlpha, B::Two) => ExtendedColorType::La2,
        (C::Rgb, B::Two) => ExtendedColorType::Rgb2,
        (C::Rgba, B::Two) => ExtendedColorType::Rgba2,
        (C::Grayscale, B::Four) => ExtendedColorType::L4,
        (C::GrayscaleAlpha, B::Four) => ExtendedColorType::La4,
        (C::Rgb, B::Four) => ExtendedColorType::Rgb4,
        (C::Rgba, B::Four) => ExtendedColorType::Rgba4,
        (C::Indexed, bits) => ExtendedColorType::Unknown(bits as u8),
    };
    Err(ImageError::Unsupported(UnsupportedError::from_format_and_kind(
        ImageFormat::Png.into(),
        UnsupportedErrorKind::Color(unsupported),
    )))
}

/// `image`'s `check_sample_format` (TIFF).
fn check_sample_format(sample_format: u16, color: tiff::ColorType) -> ImageResult<()> {
    use tiff::ColorType as Tiff;
    use tiff::tags::SampleFormat;
    let unsupported = |feature: String| {
        ImageError::Unsupported(UnsupportedError::from_format_and_kind(
            ImageFormat::Tiff.into(),
            UnsupportedErrorKind::GenericFeature(feature),
        ))
    };
    let bits = match color {
        Tiff::CMYK(k) | Tiff::Gray(k) | Tiff::RGB(k) | Tiff::RGBA(k) | Tiff::GrayA(k) => k,
        Tiff::Palette(k) | Tiff::YCbCr(k) => {
            return Err(unsupported(format!("Unhandled TIFF color type {color:?} for {k} bits")));
        }
        _ => return Err(unsupported(format!("Unhandled TIFF color type {color:?}"))),
    };
    match SampleFormat::from_u16(sample_format) {
        Some(SampleFormat::Uint) if bits <= 16 => Ok(()),
        Some(SampleFormat::IEEEFP) if bits == 32 => Ok(()),
        _ => Err(unsupported(format!("Unhandled TIFF sample format {sample_format:?} for {bits} bits"))),
    }
}

/// `image`'s `err_unknown_color_type` (TIFF).
fn tiff_unknown_color(value: u8) -> ImageError {
    ImageError::Unsupported(UnsupportedError::from_format_and_kind(
        ImageFormat::Tiff.into(),
        UnsupportedErrorKind::Color(ExtendedColorType::Unknown(value)),
    ))
}

/// `ImageError::from_png`.
fn png_error(err: png::DecodingError) -> ImageError {
    match err {
        png::DecodingError::IoError(err) => ImageError::IoError(err),
        err @ png::DecodingError::Format(_) => ImageError::Decoding(DecodingError::new(ImageFormat::Png.into(), err)),
        err @ png::DecodingError::Parameter(_) => {
            ImageError::Parameter(ParameterError::from_kind(ParameterErrorKind::Generic(err.to_string())))
        }
        png::DecodingError::LimitsExceeded => {
            ImageError::Limits(LimitError::from_kind(LimitErrorKind::InsufficientMemory))
        }
    }
}

/// `ImageError::from_decoding` (GIF).
fn gif_error(err: gif::DecodingError) -> ImageError {
    match err {
        gif::DecodingError::Io(err) => ImageError::IoError(err),
        other => ImageError::Decoding(DecodingError::new(ImageFormat::Gif.into(), other)),
    }
}

/// `ImageError::from_webp_decode`.
fn webp_error(err: image_webp::DecodingError) -> ImageError {
    match err {
        image_webp::DecodingError::IoError(err) => ImageError::IoError(err),
        _ => ImageError::Decoding(DecodingError::new(ImageFormat::WebP.into(), err)),
    }
}

/// The TIFF decoder over a file in memory.
type TiffDecoder<'a> = tiff::decoder::Decoder<Cursor<&'a [u8]>>;

/// `ImageError::from_jpeg` (zune-jpeg).
fn jpeg_error(err: zune_jpeg::errors::DecodeErrors) -> ImageError {
    use zune_jpeg::errors::DecodeErrors;
    match err {
        DecodeErrors::Unsupported(desc) => ImageError::Unsupported(UnsupportedError::from_format_and_kind(
            ImageFormat::Jpeg.into(),
            UnsupportedErrorKind::GenericFeature(format!("{desc:?}")),
        )),
        DecodeErrors::LargeDimensions(_) => ImageError::Limits(LimitError::from_kind(LimitErrorKind::DimensionError)),
        err => ImageError::Decoding(DecodingError::new(ImageFormat::Jpeg.into(), err)),
    }
}

/// `ImageError::from_tiff_decode`.
fn tiff_error(err: tiff::TiffError) -> ImageError {
    use tiff::TiffError;
    match err {
        TiffError::IoError(err) => ImageError::IoError(err),
        err @ (TiffError::FormatError(_) | TiffError::IntSizeError | TiffError::UsageError(_)) => {
            ImageError::Decoding(DecodingError::new(ImageFormat::Tiff.into(), err))
        }
        TiffError::UnsupportedError(desc) => ImageError::Unsupported(UnsupportedError::from_format_and_kind(
            ImageFormat::Tiff.into(),
            UnsupportedErrorKind::GenericFeature(desc.to_string()),
        )),
        TiffError::LimitsExceeded => ImageError::Limits(LimitError::from_kind(LimitErrorKind::InsufficientMemory)),
    }
}

#[cfg(test)]
mod tests {
    use image::codecs::gif::GifEncoder;
    use image::{DynamicImage, ImageBuffer, Rgba, RgbaImage};

    use super::*;
    use crate::decode::tiff_rgb8;

    /// How the decode was done before this module: `image::load_from_memory_with_format`, then `into_rgba8` and
    /// the flattening onto white, or `into_rgb8`.
    fn reference(bytes: &[u8], format: ImageFormat) -> Result<(usize, usize, Vec<u8>), String> {
        let img = image::load_from_memory_with_format(bytes, format).map_err(|e| e.to_string())?;
        Ok(flattened(img))
    }

    fn flattened(img: DynamicImage) -> (usize, usize, Vec<u8>) {
        let (w, h) = (img.width() as usize, img.height() as usize);
        let px = if img.color().has_alpha() {
            img.into_rgba8().pixels().flat_map(|p| [0, 1, 2].map(|c| over_white(p[c], p[3]))).collect()
        } else {
            img.into_rgb8().into_raw()
        };
        (w, h, px)
    }

    fn decoded(codecs: &mut Codecs, bytes: &[u8], format: Format) -> Result<(usize, usize, Vec<u8>), String> {
        // A buffer left dirty by an earlier image, as a worker's is.
        let mut out = Rgb { width: 0, height: 0, pixels: vec![0xA5; 1000] };
        codecs.decode(bytes, format, &mut out).map_err(|e| e.to_string())?;
        assert_eq!(out.pixels.len(), out.width * out.height * 3);
        Ok((out.width, out.height, out.pixels))
    }

    fn ne_bytes<const N: usize, T: Copy>(values: &[T], to: impl Fn(T) -> [u8; N]) -> Vec<u8> {
        values.iter().flat_map(|&v| to(v)).collect()
    }

    /// Each colour type's samples, every value of every channel (16-bit included), against `image`'s conversion.
    #[test]
    fn conversions_are_images() {
        let check = |raw: Vec<u8>, color: ColorType, img: DynamicImage| {
            let (w, h, want) = flattened(img);
            let mut out = Rgb::default();
            convert(&raw, color, None, w * h, &mut out).unwrap();
            assert!(out.pixels == want, "{color:?}");
        };
        // Every 8-bit pair (value, alpha); every 16-bit value, with alphas across their range.
        let pairs8: Vec<[u8; 2]> = (0..=255).flat_map(|a| (0..=255).map(move |v| [v, a])).collect();
        let v16: Vec<u16> = (0..=u16::MAX).collect();
        let pairs16: Vec<[u16; 2]> = v16.iter().map(|&v| [v, v.wrapping_mul(40503)]).collect();
        let n8 = pairs8.len() as u32;
        let n16 = v16.len() as u32;

        let l8: Vec<u8> = pairs8.iter().map(|p| p[0]).collect();
        check(l8.clone(), ColorType::L8, DynamicImage::ImageLuma8(ImageBuffer::from_raw(n8, 1, l8).unwrap()));
        let la8: Vec<u8> = pairs8.iter().flatten().copied().collect();
        check(la8.clone(), ColorType::La8, DynamicImage::ImageLumaA8(ImageBuffer::from_raw(n8, 1, la8).unwrap()));
        let rgba8: Vec<u8> = pairs8.iter().flat_map(|&[v, a]| [v, v ^ 0x5A, 255 - v, a]).collect();
        check(rgba8.clone(), ColorType::Rgba8, DynamicImage::ImageRgba8(ImageBuffer::from_raw(n8, 1, rgba8).unwrap()));
        let rgb8: Vec<u8> = pairs8.iter().flat_map(|&[v, a]| [v, a, v ^ a]).collect();
        check(rgb8.clone(), ColorType::Rgb8, DynamicImage::ImageRgb8(ImageBuffer::from_raw(n8, 1, rgb8).unwrap()));

        let l16 = DynamicImage::ImageLuma16(ImageBuffer::from_raw(n16, 1, v16.clone()).unwrap());
        check(ne_bytes(&v16, u16::to_ne_bytes), ColorType::L16, l16);
        let la16: Vec<u16> = pairs16.iter().flatten().copied().collect();
        let la16_bytes = ne_bytes(&la16, u16::to_ne_bytes);
        check(la16_bytes, ColorType::La16, DynamicImage::ImageLumaA16(ImageBuffer::from_raw(n16, 1, la16).unwrap()));
        let rgb16: Vec<u16> = v16.iter().flat_map(|&v| [v, !v, v.rotate_left(5)]).collect();
        let rgb16_bytes = ne_bytes(&rgb16, u16::to_ne_bytes);
        check(rgb16_bytes, ColorType::Rgb16, DynamicImage::ImageRgb16(ImageBuffer::from_raw(n16, 1, rgb16).unwrap()));
        let rgba16: Vec<u16> = pairs16.iter().flat_map(|&[v, a]| [v, !v, v.rotate_left(5), a]).collect();
        let rgba16_bytes = ne_bytes(&rgba16, u16::to_ne_bytes);
        let rgba16 = DynamicImage::ImageRgba16(ImageBuffer::from_raw(n16, 1, rgba16).unwrap());
        check(rgba16_bytes, ColorType::Rgba16, rgba16);

        // Floats: the edges (NaN, infinities, signed zeros, out of range, each rounding boundary and its
        // neighbours), then a million spread over [-0.5, 1.5].
        let mut floats = vec![f32::NAN, -f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 0.0, -0.0, -1.0, 2.0, 1.0];
        floats.extend([f32::MIN_POSITIVE, -f32::MIN_POSITIVE, 1.0f32.next_up(), 1.0f32.next_down()]);
        for k in 0..=255 {
            let edge = (k as f32 + 0.5) / 255.0;
            floats.extend([edge.next_down(), edge, edge.next_up()]);
        }
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        floats.extend((0..1_000_000).map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 40) as f32 / (1u64 << 24) as f32 * 2.0 - 0.5
        }));
        // Whole pixels of both 3 and 4 channels.
        floats.truncate(floats.len() / 12 * 12);
        let n = (floats.len() / 12) as u32;
        let float_bytes = ne_bytes(&floats, f32::to_ne_bytes);
        let rgb = DynamicImage::ImageRgb32F(ImageBuffer::from_raw(n * 4, 1, floats.clone()).unwrap());
        check(float_bytes.clone(), ColorType::Rgb32F, rgb);
        let rgba = DynamicImage::ImageRgba32F(ImageBuffer::from_raw(n * 3, 1, floats).unwrap());
        check(float_bytes, ColorType::Rgba32F, rgba);
    }

    /// Planes give what the same samples interleaved give.
    #[test]
    fn planes_convert_as_interleaved() {
        let pixels = 300;
        let chunky = noise(pixels * 4 * 2, 5);
        let mut planar = vec![0; chunky.len()];
        for i in 0..pixels {
            for c in 0..4 {
                planar[(c * pixels + i) * 2..][..2].copy_from_slice(&chunky[(i * 4 + c) * 2..][..2]);
            }
        }
        let (mut a, mut b) = (Rgb::default(), Rgb::default());
        convert(&chunky, ColorType::Rgba16, None, pixels, &mut a).unwrap();
        convert(&planar, ColorType::Rgba16, Some(pixels * 2), pixels, &mut b).unwrap();
        assert!(a.pixels == b.pixels);
    }

    /// A test pattern: gradients and noise, with an alpha ramp.
    fn pattern(w: u32, h: u32, seed: u32) -> RgbaImage {
        RgbaImage::from_fn(w, h, |x, y| {
            let n = (x.wrapping_mul(2654435761) ^ y.wrapping_mul(40503) ^ seed).wrapping_mul(2246822519) >> 24;
            Rgba([(x * 255 / w) as u8, (y * 255 / h) as u8, n as u8, ((x + y) * 255 / (w + h)) as u8])
        })
    }

    fn noise(len: usize, seed: usize) -> Vec<u8> {
        (0..len).map(|i| ((i ^ seed).wrapping_mul(2654435761) >> 13) as u8).collect()
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
        let offsets: Vec<u32> = (0..planes).map(|p| (out.len() + p * strip) as u32).collect();
        out.extend_from_slice(samples);
        // (tag, type, values): type 3 is SHORT, 4 LONG.
        let mut entries: Vec<(u16, u16, Vec<u32>)> = vec![
            (256, 4, vec![w]),
            (257, 4, vec![h]),
            (258, 3, vec![u32::from(kind.bits); count]),
            (259, 3, vec![1]),
            (262, 3, vec![u32::from(kind.photometric)]),
            (273, 4, offsets),
            (277, 3, vec![count as u32]),
            (278, 4, vec![h]),
            (279, 4, vec![strip as u32; planes]),
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
                    .flat_map(
                        |&v| if *kind == 3 { (v as u16).to_le_bytes().to_vec() } else { v.to_le_bytes().to_vec() },
                    )
                    .collect()
            })
            .collect();
        let ifd_at = out.len() + encoded.iter().filter(|b| b.len() > 4).map(Vec::len).sum::<usize>();
        for ((tag, kind, values), mut bytes) in entries.iter().zip(encoded) {
            ifd.extend(tag.to_le_bytes());
            ifd.extend(kind.to_le_bytes());
            ifd.extend((values.len() as u32).to_le_bytes());
            if bytes.len() <= 4 {
                bytes.resize(4, 0);
                ifd.extend(bytes);
            } else {
                ifd.extend(((out.len() + arrays.len()) as u32).to_le_bytes());
                arrays.extend(bytes);
            }
        }
        out.extend(arrays);
        assert_eq!(out.len(), ifd_at);
        out[4..8].copy_from_slice(&(ifd_at as u32).to_le_bytes());
        out.extend((entries.len() as u16).to_le_bytes());
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
            (png::ColorType::Grayscale, &[1u8, 2, 4, 8, 16][..]),
            (png::ColorType::GrayscaleAlpha, &[8, 16]),
            (png::ColorType::Rgb, &[8, 16]),
            (png::ColorType::Rgba, &[8, 16]),
            (png::ColorType::Indexed, &[1, 2, 4, 8]),
        ] {
            for &depth in depths {
                for trns in [false, true] {
                    if trns && matches!(ct, png::ColorType::GrayscaleAlpha | png::ColorType::Rgba) {
                        continue;
                    }
                    let samples = match ct {
                        png::ColorType::Grayscale | png::ColorType::Indexed => 1,
                        png::ColorType::GrayscaleAlpha => 2,
                        png::ColorType::Rgb => 3,
                        _ => 4,
                    };
                    let data = noise((w as usize * samples * depth as usize).div_ceil(8) * h as usize, depth as usize);
                    let mut out = Vec::new();
                    {
                        let mut e = png::Encoder::new(&mut out, w, h);
                        e.set_color(ct);
                        e.set_depth(png::BitDepth::from_u8(depth).unwrap());
                        if ct == png::ColorType::Indexed {
                            let n = 1usize << depth;
                            e.set_palette((0..n * 3).map(|i| (i * 37) as u8).collect::<Vec<_>>());
                            if trns {
                                e.set_trns((0..n).map(|i| (i * 91) as u8).collect::<Vec<_>>());
                            }
                        } else if trns {
                            let v = if depth == 16 { vec![0, 40] } else { vec![0, 40 & ((1u16 << depth) - 1) as u8] };
                            e.set_trns(if ct == png::ColorType::Rgb { v.repeat(3) } else { v });
                        }
                        e.write_header().unwrap().write_image_data(&data).unwrap();
                    }
                    files.push((format!("png {ct:?} {depth} trns {trns}"), Format::Png, ImageFormat::Png, out));
                }
            }
        }
        {
            let mut out = Vec::new();
            let mut e = png::Encoder::new(&mut out, w, h);
            e.set_color(png::ColorType::Rgba);
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
                let mut e = gif::Encoder::new(&mut out, w as u16, h as u16, &[]).unwrap();
                for (i, (left, top, fw, fh)) in frames.into_iter().enumerate() {
                    let mut rgba = pattern(fw, fh, i as u32 + 1).into_raw();
                    let mut frame = gif::Frame::from_rgba_speed(fw as u16, fh as u16, &mut rgba, 10);
                    (frame.left, frame.top) = (left as u16, top as u16);
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
        let pixels = (w * h) as usize;
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
        let samples = noise((w as usize).div_ceil(8) * h as usize, 1);
        files.push(("tiff gray 1".into(), Format::Tiff, ImageFormat::Tiff, tiff_file(w, h, &gray1, &samples)));
        // One RGB8 image, classic and BigTIFF, as the tiff crate's encoder writes them.
        let rgb = noise(pixels * 3, 8);
        files.push(("tiff rgb8 encoder".into(), Format::Tiff, ImageFormat::Tiff, tiff_rgb8(w, h, &rgb, false)));
        files.push(("tiff rgb8 BigTIFF".into(), Format::Tiff, ImageFormat::Tiff, tiff_rgb8(w, h, &rgb, true)));
        let cmyk: Vec<u8> = pattern(w, h, 9).as_raw().iter().map(|v| 255 - v).collect();
        for (name, format, pixels, sub) in [
            ("rgb", turbojpeg::PixelFormat::RGBA, pattern(w, h, 3).into_raw(), turbojpeg::Subsamp::Sub2x2),
            ("gray", turbojpeg::PixelFormat::GRAY, rgba.to_luma8().into_raw(), turbojpeg::Subsamp::Gray),
            ("cmyk", turbojpeg::PixelFormat::CMYK, cmyk, turbojpeg::Subsamp::None),
        ] {
            let pitch = w as usize * format.size();
            let image = turbojpeg::Image { pixels: &pixels[..], width: w as usize, pitch, height: h as usize, format };
            let jpeg = turbojpeg::compress(image, 90, sub).unwrap().to_vec();
            files.push((format!("jpeg {name}"), Format::Jpeg, ImageFormat::Jpeg, jpeg));
        }
        files
    }

    /// Every kind of file decodes to what `image` gave, or fails with its error, with one `Codecs` reused across
    /// them all, forwards and backwards, so that state left by one file would show in another.
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
                out.push(chunk.len() as u8);
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
        let mut chunk = (data.len() as u32).to_be_bytes().to_vec();
        chunk.extend(kind);
        chunk.extend(data);
        chunk.extend(crc32fast::hash(&[&kind[..], data].concat()).to_be_bytes());
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
