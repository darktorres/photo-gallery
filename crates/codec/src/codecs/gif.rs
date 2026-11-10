//! GIF's first frame, through the gif crate as `image`'s `GifDecoder` drives it.

use std::io::Cursor;

use image::error::{DecodingError, LimitError, LimitErrorKind, ParameterError, ParameterErrorKind};
use image::{ColorType, ImageError, ImageFormat, ImageResult};

use super::convert::{finish, zeroed};
use super::limits::reserve;
use crate::pixels::Rgb;
use pg_model::num;

/// `image`'s `GifDecoder::new` and `read_image`: the first frame, as RGBA. In the buffers of the GIF before
/// (vendored gif's `Scratch`), which go back to `scratch` whatever happens, and read straight from `bytes`, where
/// `image` reads them through a `BufReader`. A frame that doesn't span the image's width is read into `raw` first.
pub(super) fn decode(scratch: &mut gif::Scratch, raw: &mut Vec<u8>, bytes: &[u8], out: &mut Rgb) -> ImageResult<()> {
    let mut options = gif::DecodeOptions::new();
    options.set_color_output(gif::ColorOutput::RGBA);
    // The XMP and ICC extensions are skipped, where `image`'s decoder collects them for its metadata getters,
    // which a decode doesn't call: the metadata reader finds the XMP itself, and colours aren't managed. That
    // changes no pixel and no error, except for a file whose XMP or ICC alone is over gif's 50 MB memory limit,
    // which `image` fails and this decodes.
    options.skip_metadata(true);
    let mut reader = match options.read_info_with(Cursor::new(bytes), std::mem::take(scratch)) {
        Ok(reader) => reader,
        Err((err, kept)) => {
            *scratch = kept;
            return Err(gif_error(err));
        }
    };
    let result = read(&mut reader, raw, out);
    *scratch = reader.into_scratch();
    result
}

/// [`decode`]'s image, once the headers are read.
fn read(reader: &mut gif::Decoder<Cursor<&[u8]>, Cursor<&[u8]>>, raw: &mut Vec<u8>, out: &mut Rgb) -> ImageResult<()> {
    let (w, h) = (u32::from(reader.width()), u32::from(reader.height()));
    let mut limits = reserve(ColorType::Rgba8, w, h)?;
    let buf = zeroed(&mut out.pixels, num::index(w) * num::index(h) * 4);
    let Some(frame) = reader.next_frame_info().map_err(gif_error)? else {
        return Err(ImageError::Parameter(ParameterError::from_kind(ParameterErrorKind::NoMoreData)));
    };
    let [left, top, fw, fh] = [frame.left, frame.top, frame.width, frame.height].map(u32::from);
    let line = num::index(w) * 4;
    if left == 0
        && fw == w
        && u64::from(top) + u64::from(fh) <= u64::from(h)
        && let Some(rows) = buf.get_mut(line * num::index(top)..).and_then(|rows| rows.get_mut(..line * num::index(fh)))
    {
        // A frame that spans the image's width is read in place; the rows above and below it stay zero.
        reader.read_into_buffer(rows).map_err(gif_error)?;
    } else {
        // Any other frame is read on its own, then placed: what it doesn't cover stays zero (transparent).
        let size = num::index(fw)
            .checked_mul(num::index(fh))
            .and_then(|s| s.checked_mul(4))
            .ok_or(ImageError::Limits(LimitError::from_kind(LimitErrorKind::InsufficientMemory)))?;
        limits.reserve_usize(size)?;
        let frame = zeroed(raw, size);
        limits.free_usize(size);
        reader.read_into_buffer(frame).map_err(gif_error)?;
        // The frame's rows from row `top`, its pixels from column `left`, as far as the image goes. (A side of
        // zero has no rows to place.)
        let image_rows = buf.chunks_exact_mut(line.max(1)).skip(num::index(top));
        for (row, frame_row) in image_rows.zip(frame.chunks_exact((num::index(fw) * 4).max(1))) {
            let pixels = row.as_chunks_mut::<4>().0.iter_mut().skip(num::index(left));
            for (d, s) in pixels.zip(frame_row.as_chunks::<4>().0) {
                *d = *s;
            }
        }
    }
    finish(ColorType::Rgba8, w, h, out, raw)
}

/// `ImageError::from_decoding` (GIF).
fn gif_error(err: gif::DecodingError) -> ImageError {
    match err {
        gif::DecodingError::Io(err) => ImageError::IoError(err),
        other => ImageError::Decoding(DecodingError::new(ImageFormat::Gif.into(), other)),
    }
}
