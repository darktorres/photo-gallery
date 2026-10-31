//! Animated GIF, APNG and WebP, frame by frame, for the viewer's player. `image`'s decoders composite each frame
//! onto the canvas (disposal and blending), so every frame comes out whole. FFmpeg decodes all three too, but its
//! animated WebP demuxer can't seek back to the start, so Qt's player can't loop those.

use std::io::Cursor;
use std::sync::Arc;

use anyhow::{Result, bail};
use image::AnimationDecoder;
use image::codecs::gif::GifDecoder;
use image::codecs::png::PngDecoder;
use image::codecs::webp::WebPDecoder;

use crate::decode::Format;

/// A whole frame, RGBA (not premultiplied), shown for `delay_ms`.
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub delay_ms: u32,
}

/// Delays below this are shown for [`SHORT_DELAY_MS`], as browsers do: many GIFs say 0 and mean "the default".
const MIN_DELAY_MS: u32 = 20;
const SHORT_DELAY_MS: u32 = 100;

/// One pass over the frames of the animation in `bytes` (a file's content). Loop by calling it again.
pub fn frames(bytes: Arc<[u8]>) -> Result<impl Iterator<Item = Result<Frame>>> {
    let format = Format::sniff(&bytes);
    let cursor = Cursor::new(bytes);
    let frames = match format {
        Some(Format::Gif) => GifDecoder::new(cursor)?.into_frames(),
        Some(Format::Png) => PngDecoder::new(cursor)?.apng()?.into_frames(),
        Some(Format::Webp) => WebPDecoder::new(cursor)?.into_frames(),
        _ => bail!("not a GIF, PNG or WebP file"),
    };
    Ok(frames.map(|frame| {
        let frame = frame?;
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

        assert!(frames(Arc::from(&b"not an image"[..])).is_err());
    }
}
