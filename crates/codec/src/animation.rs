//! Animated GIF, APNG and WebP, frame by frame, for the viewer's player. `image`'s decoders composite each frame
//! onto the canvas (disposal and blending), so every frame comes out whole. FFmpeg decodes all three too, but its
//! animated WebP demuxer can't seek back to the start, so Qt's player can't loop those.

use std::io::Cursor;
use std::sync::Arc;

use crate::error::{Error, Result};
use image::codecs::gif::GifDecoder;
use image::codecs::png::PngDecoder;
use image::codecs::webp::WebPDecoder;
use image::{AnimationDecoder, ImageDecoder, Limits};

use pg_model::format::Format;

/// A whole frame, RGBA (not premultiplied), shown for `delay_ms`.
pub struct Frame {
    /// The frame's width, in pixels.
    pub width: u32,
    /// The frame's height, in pixels.
    pub height: u32,
    /// The pixels, RGBA and not premultiplied, row after row.
    pub rgba: Vec<u8>,
    /// How long the frame is shown, in milliseconds.
    pub delay_ms: u32,
}

/// Delays below this are shown for [`SHORT_DELAY_MS`], as browsers do: many GIFs say 0 and mean "the default".
const MIN_DELAY_MS: u32 = 20;
const SHORT_DELAY_MS: u32 = 100;

/// The decoder with the same limits as the still decodes (`convert::read_with`): its canvas is held to the cap for
/// decoded buffers before anything that size is allocated, so a damaged header is an error, not an allocation failure.
fn limited<D: ImageDecoder>(mut decoder: D) -> Result<D> {
    let mut limits = Limits::default();
    limits.reserve(decoder.total_bytes()).map_err(Error::decoder)?;
    decoder.set_limits(limits).map_err(Error::decoder)?;
    Ok(decoder)
}

/// The bytes of an animation: shared and read-only, so a player holds them without a copy. Made from what holds them (a
/// file's read, a search's kept download), by [`Bytes::shared`] or `From<Vec<u8>>`.
#[derive(Clone)]
pub struct Bytes(Arc<dyn AsRef<[u8]> + Send + Sync>);

impl Bytes {
    /// The bytes `bytes` holds, shared: no copy.
    pub fn shared<T: AsRef<[u8]> + Send + Sync + 'static>(bytes: Arc<T>) -> Bytes {
        Bytes(bytes)
    }
}

impl From<Vec<u8>> for Bytes {
    fn from(bytes: Vec<u8>) -> Bytes {
        Bytes(Arc::new(bytes))
    }
}

impl AsRef<[u8]> for Bytes {
    fn as_ref(&self) -> &[u8] {
        (*self.0).as_ref()
    }
}

/// One pass over the frames of the animation in `bytes` (a file's content). Loop by calling it again.
pub fn frames(bytes: Bytes) -> Result<impl Iterator<Item = Result<Frame>>> {
    let format = Format::sniff(bytes.as_ref());
    let cursor = Cursor::new(bytes);
    let frames = match format {
        Some(Format::Gif) => limited(GifDecoder::new(cursor).map_err(Error::decoder)?)?.into_frames(),
        Some(Format::Png) => {
            limited(PngDecoder::new(cursor).map_err(Error::decoder)?)?.apng().map_err(Error::decoder)?.into_frames()
        }
        Some(Format::Webp) => limited(WebPDecoder::new(cursor).map_err(Error::decoder)?)?.into_frames(),
        _ => return Err(Error::invalid("not a GIF, PNG or WebP file")),
    };
    Ok(frames.map(|frame| -> Result<Frame> {
        let frame = frame.map_err(Error::decoder)?;
        let (numer, denom) = frame.delay().numer_denom_ms();
        let delay_ms = numer / denom.max(1);
        let rgba = frame.into_buffer();
        Ok(Frame {
            width: rgba.width(),
            height: rgba.height(),
            rgba: rgba.into_raw(),
            delay_ms: if delay_ms < MIN_DELAY_MS { SHORT_DELAY_MS } else { delay_ms },
        })
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gif_and_apng_frames() {
        use image::codecs::gif::GifEncoder;
        let mut gif = Vec::new();
        {
            let mut enc = GifEncoder::new(&mut gif);
            for (i, delay) in [(0u8, 0u32), (1, 50), (2, 250)] {
                let img = image::RgbaImage::from_pixel(4, 2, image::Rgba([i * 100, 0, 0, 255]));
                enc.encode_frame(image::Frame::from_parts(img, 0, 0, image::Delay::from_numer_denom_ms(delay, 1)))
                    .unwrap();
            }
        }
        let got: Vec<Frame> = frames(gif.into()).unwrap().collect::<Result<_>>().unwrap();
        let summary: Vec<_> = got.iter().map(|f| (f.width, f.height, f.rgba[0], f.delay_ms)).collect();
        assert_eq!(summary, [(4, 2, 0, SHORT_DELAY_MS), (4, 2, 100, 50), (4, 2, 200, 250)]);

        let mut apng = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut apng, 2, 2);
            enc.set_color(png::ColorType::Rgba);
            enc.set_animated(2, 0).unwrap();
            enc.set_frame_delay(1, 25).unwrap(); // 40 ms
            let mut w = enc.write_header().unwrap();
            w.write_image_data(&[9; 16]).unwrap();
            w.write_image_data(&[200; 16]).unwrap();
            w.finish().unwrap();
        }
        let got: Vec<Frame> = frames(apng.into()).unwrap().collect::<Result<_>>().unwrap();
        let summary: Vec<_> = got.iter().map(|f| (f.width, f.height, f.rgba[0], f.delay_ms)).collect();
        assert_eq!(summary, [(2, 2, 9, 40), (2, 2, 200, 40)]);

        assert!(frames(b"not an image".to_vec().into()).is_err());
    }

    // A GIF whose logical screen (its canvas) is 40,000 x 40,000 is over the cap, and has no frames: it is refused
    // before the canvas is allocated, not read as an animation of no frames.
    #[test]
    fn a_gif_canvas_over_the_cap_is_refused() {
        let mut gif = b"GIF89a".to_vec();
        gif.extend_from_slice(&40_000u16.to_le_bytes());
        gif.extend_from_slice(&40_000u16.to_le_bytes());
        gif.extend_from_slice(&[0x00, 0x00, 0x00, 0x3B]);
        assert!(frames(gif.into()).is_err());
    }
}
