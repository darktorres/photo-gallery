//! What a file shows as: its pixels turned upright (the EXIF orientation, then the catalog's quarter turns), decoded
//! at the smallest size a caller needs. The one place that reads a file's EXIF orientation for decoding, so the codec
//! ([`pg_codec::decode`]) takes the orientation as a value and reads no metadata itself.
//!
//! The viewer's pipeline is here too: the decode a [`View`] asks for ([`min_size`]), a kept decode reused while it is
//! big enough ([`Shown::covers`]), the fit and the zoomed region ([`Shown::resized`]), a video's poster frame
//! ([`poster`]) and the ThumbHash placeholder ([`placeholder`]), in the format the window uploads ([`Upload`]).

use std::path::Path;

use anyhow::{Context, Result};
use pg_codec::decode::{Decoded, Decoder, Info, MinSize, Stored, orient_pixels};
use pg_codec::pixels::Rgb;
use pg_codec::resize::Resizer;
use pg_model::format::Format;
use pg_model::image::View;
use pg_model::num::count;
use pg_model::orientation::Orientation;
use pg_model::turns::Turns;

/// Lanczos3 from a DCT-scaled decode keeping ≥ 4× the target matches a full decode (FINDINGS "Thumbnails"): the
/// thumbnails' decode and the viewer's keep this margin.
pub(crate) const DECODE_MARGIN: usize = 4;

/// Decode `bytes` upright at the smallest DCT scale that keeps `min` (full size if the image is smaller) into a
/// worker's buffers, which keep their capacity from file to file: the pixels go into `image`, oriented through
/// `spare`. `turns` is the catalog rotation (`files.rotation`, quarter turns clockwise), applied after the EXIF
/// orientation.
pub fn decode_into(
    decoder: &mut Decoder,
    bytes: &[u8],
    min: MinSize,
    turns: Turns,
    image: &mut Rgb,
    spare: &mut Vec<u8>,
) -> Result<Info> {
    let format = Format::sniff(bytes).context("unknown image format")?;
    let exif = pg_metadata::orientation(bytes, format);
    decoder.decode_into(bytes, format, exif, min, turns, image, spare)
}

/// [`decode_into`] into new buffers: a one-off decode.
pub fn decode(decoder: &mut Decoder, bytes: &[u8], min: MinSize, turns: Turns) -> Result<Decoded> {
    let mut image = Rgb::default();
    let info = decode_into(decoder, bytes, min, turns, &mut image, &mut Vec::new())?;
    Ok(Decoded { image, info })
}

/// [`decode`] without the orientation, into new buffers: the pixels as stored, which is what a lossless crop cuts.
pub fn decode_stored(decoder: &mut Decoder, bytes: &[u8], min: MinSize, turns: Turns) -> Result<Stored> {
    let format = Format::sniff(bytes).context("unknown image format")?;
    let exif = pg_metadata::orientation(bytes, format);
    let mut image = Rgb::default();
    let info = decoder.decode_stored_into(bytes, format, exif, min, turns, &mut image)?;
    Ok(Stored { image, info })
}

/// The least the viewer decodes for `view`: enough that the image it shows keeps 4× its pixels (the margin the thumbnails keep too).
pub fn min_size(view: View) -> MinSize {
    match view {
        View::Fit(w, h) => MinSize::Fit(w * DECODE_MARGIN, h * DECODE_MARGIN),
        View::Full => MinSize::Full,
        // The whole image at a scale where the region keeps ≥ DECODE_MARGIN× the output pixels.
        View::Region((_, _, rw, rh), w, h) => MinSize::Fit(
            to_pixels((count(w) * count(DECODE_MARGIN) / rw.max(1e-6)).ceil()),
            to_pixels((count(h) * count(DECODE_MARGIN) / rh.max(1e-6)).ceil()),
        ),
    }
}

/// `x` rounded toward zero, as a count of pixels.
fn to_pixels(x: f64) -> usize {
    pg_model::num::f64_to_usize(x.clamp(0.0, f64::from(u32::MAX)))
}

/// A decode as the viewer keeps it: upright, possibly DCT-scaled, with the upright full size of the image it is of.
pub struct Shown {
    pub(crate) full: (usize, usize),
    pub(crate) image: Rgb,
}

impl From<Decoded> for Shown {
    fn from(decoded: Decoded) -> Shown {
        let full = (pg_model::num::index(decoded.info.full_width), pg_model::num::index(decoded.info.full_height));
        Shown { full, image: decoded.image }
    }
}

impl Shown {
    /// Whether this decode is enough for `view` ([`min_size`]): it is the full size, or at least as large as the
    /// decode the view asks for.
    pub fn covers(&self, view: View) -> bool {
        let (w, h, full) = (self.image.width, self.image.height, self.full);
        if (w, h) == full {
            return true;
        }
        match min_size(view) {
            MinSize::Full => false,
            MinSize::Fit(bw, bh) => {
                let scale = (count(bw) / count(full.0)).min(count(bh) / count(full.1)).min(1.0);
                count(w) >= (count(full.0) * scale).floor() && count(h) >= (count(full.1) * scale).floor()
            }
            MinSize::Upright(mw, mh) => w >= mw.min(full.0) && h >= mh.min(full.1),
            MinSize::Sides(long, short) => {
                w.max(h) >= long.min(full.0.max(full.1)) && w.min(h) >= short.min(full.0.min(full.1))
            }
        }
    }

    /// The image the viewer shows for `view`, resampled from this decode: fitted into the window, or the zoomed region
    /// at the window's size. `None` for [`View::Full`], which shows the decode as it is.
    pub fn resized(&self, view: View) -> Result<Option<Rgb>> {
        let img = &self.image;
        let mut out = Rgb::default();
        match view {
            View::Fit(w, h) => Resizer::new().fit_box(img, w, h, &mut out)?,
            View::Full => return Ok(None),
            View::Region((x, y, rw, rh), w, h) => {
                let (iw, ih) = (count(img.width), count(img.height));
                Resizer::new().resize_region(img, (x * iw, y * ih, rw * iw, rh * ih), w, h, &mut out)?;
            }
        }
        Ok(Some(out))
    }

    /// What the viewer shows for `view` ([`Shown::resized`]), in the upload format.
    pub fn upload(&self, view: View) -> Result<Upload> {
        Ok(match self.resized(view)? {
            Some(resized) => Upload::opaque(&resized),
            None => Upload::opaque(&self.image),
        })
    }
}

/// `bytes` upright at full size, as the viewer keeps it: a thumbnail turned for a turn not yet written into its file.
pub fn decode_full(decoder: &mut Decoder, bytes: &[u8], turns: Turns) -> Result<Shown> {
    decode(decoder, bytes, MinSize::Full, turns).map(Shown::from)
}

/// The poster frame of the video at `path` (`at_ms` into it), upright: what the viewer shows of a video until it plays.
pub fn poster(path: &Path, at_ms: i64, turns: Turns) -> Result<Shown> {
    let frame = pg_video::frame(path, at_ms, turns)?;
    Ok(Shown { full: (frame.width, frame.height), image: frame })
}

/// The ThumbHash placeholder `hash` as a tiny image, turned by `turns`.
pub fn placeholder(hash: &[u8], turns: Turns) -> Result<Upload> {
    let (w, h, rgba) = thumbhash::thumb_hash_to_rgba(hash).ok().context("bad thumbhash")?;
    let (rgba, w, h) = orient_pixels(rgba, w, h, 4, Orientation::UPRIGHT.compose(turns));
    Ok(Upload::straight(w, h, rgba))
}

/// An image in a format Qt's scene graph uploads as it is (`QSGPlainTexture`): 4 bytes a pixel, RGBX8888 or RGBA8888
/// premultiplied. Any other format it converts on the render thread, a copy of the whole image at every upload.
pub struct Upload {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u8>,
    /// RGBA8888 premultiplied; else RGBX8888 (X = 255).
    pub premultiplied: bool,
}

impl Upload {
    /// `img` as RGBX8888.
    pub fn opaque(img: &Rgb) -> Upload {
        let mut pixels = vec![255u8; img.width * img.height * 4];
        for (out, rgb) in pixels.as_chunks_mut::<4>().0.iter_mut().zip(img.pixels.as_chunks::<3>().0) {
            out[..3].copy_from_slice(rgb);
        }
        Upload { width: img.width, height: img.height, pixels, premultiplied: false }
    }

    /// Straight RGBA as RGBA8888 premultiplied, in place, rounded as Qt premultiplies (`qPremultiply`), so the texture is
    /// the one Qt's own conversion made.
    pub fn straight(width: usize, height: usize, mut pixels: Vec<u8>) -> Upload {
        for px in pixels.as_chunks_mut::<4>().0 {
            let a = u32::from(px[3]);
            if a != 255 {
                for c in &mut px[..3] {
                    let t = u32::from(*c) * a;
                    *c = u8::try_from((t + (t >> 8) + 0x80) >> 8).unwrap_or(u8::MAX);
                }
            }
        }
        Upload { width, height, pixels, premultiplied: true }
    }
}

#[cfg(test)]
mod tests {
    use pg_model::turns::Turns;

    #[test]
    fn upright_follows_the_decoders_transform() {
        // A 4×2 stored image whose only lit pixel block is the rectangle x 1..3, y 0..1; every orientation and turn
        // must put the rectangle where decode::orient (and then quarter turns) puts the pixels.
        let (w, h) = (4usize, 2usize);
        let lit = |x: usize, y: usize| (1..3).contains(&x) && y < 1;
        let pixels: Vec<u8> = (0..h).flat_map(|y| (0..w).map(move |x| u8::from(lit(x, y)))).collect();
        let rect = [1.0 / 4.0, 0.0, 3.0 / 4.0, 1.0 / 2.0];
        for tag in 1..=8u8 {
            let orientation = pg_model::orientation::Orientation::new(tag).unwrap();
            for turns in 0..4u8 {
                let (mut px, mut pw, mut ph) = pg_codec::decode::transform(&pixels, w, h, 1, orientation);
                for _ in 0..turns {
                    (px, pw, ph) = pg_codec::decode::transform(
                        &px,
                        pw,
                        ph,
                        1,
                        pg_model::orientation::Orientation::new(6).unwrap(),
                    );
                }
                let [x0, y0, x1, y1] = orientation.upright_rect(rect, Turns::new(turns));
                for y in 0..ph {
                    for x in 0..pw {
                        let (cx, cy) = (
                            (pg_model::num::count(x) + 0.5) / pg_model::num::count(pw),
                            (pg_model::num::count(y) + 0.5) / pg_model::num::count(ph),
                        );
                        let inside = (x0..x1).contains(&cx) && (y0..y1).contains(&cy);
                        assert_eq!(px[y * pw + x] == 1, inside, "orientation {tag}, {turns} turns, ({x}, {y})");
                    }
                }
            }
        }
    }

    /// A decode covers a view when it is the full size, or at least the decode the view asks for.
    #[test]
    fn a_decode_covers_the_views_it_is_big_enough_for() {
        use super::{Rgb, Shown, View};
        let shown = |w, h| Shown { full: (4000, 3000), image: Rgb { width: w, height: h, pixels: Vec::new() } };
        assert!(shown(4000, 3000).covers(View::Full), "the full size covers everything");
        assert!(!shown(1000, 750).covers(View::Full));
        // A 250×200 window asks for 4× that: 1000×800, which the 4000×3000 image fits into at 1000×750.
        assert!(shown(1000, 750).covers(View::Fit(250, 200)));
        assert!(!shown(500, 375).covers(View::Fit(250, 200)));
    }
}
