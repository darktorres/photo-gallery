//! What a file shows as: its pixels turned upright (the EXIF orientation, then the catalog's quarter turns), decoded
//! at the smallest size a caller needs. The one place that reads a file's EXIF orientation for decoding, so the codec
//! ([`pg_codec::decode`]) takes the orientation as a value and reads no metadata itself.

use anyhow::{Context, Result};

use pg_codec::decode::{Decoded, Decoder, MinSize, Stored};
use pg_model::format::Format;
use pg_model::orientation;

/// Decode `bytes` upright at the smallest DCT scale that keeps `min` (full size if the image is smaller), into new
/// buffers: a one-off decode (the viewer). `turns` is the catalog rotation (`files.rotation`, quarter turns clockwise),
/// applied after the EXIF orientation.
pub fn decode(decoder: &mut Decoder, bytes: &[u8], min: MinSize, turns: u8) -> Result<Decoded> {
    let format = Format::sniff(bytes).context("unknown image format")?;
    let exif = pg_metadata::orientation(bytes, format);
    let mut image = pg_codec::pixels::Rgb::default();
    let info = decoder.decode_into(bytes, format, exif, min, turns, &mut image, &mut Vec::new())?;
    Ok(Decoded { image, info })
}

/// [`decode`] without the orientation, into new buffers: the pixels as stored, which is what a lossless crop cuts.
pub fn decode_stored(decoder: &mut Decoder, bytes: &[u8], min: MinSize, turns: u8) -> Result<Stored> {
    let format = Format::sniff(bytes).context("unknown image format")?;
    let exif = pg_metadata::orientation(bytes, format);
    let mut image = pg_codec::pixels::Rgb::default();
    let info = decoder.decode_stored_into(bytes, format, exif, min, turns, &mut image)?;
    Ok(Stored { image, info })
}

/// A rectangle of the stored image (fractions: left, top, right, bottom) in the image as shown: turned upright by
/// `orientation` (1–8, as `decode::orient` applies it), then `turns` quarter turns clockwise.
pub fn upright(rect: [f64; 4], orientation: u8, turns: u8) -> [f64; 4] {
    // Where a stored point (u, v) is shown, as decode::transform moves pixels.
    let shown = |(u, v): (f64, f64)| orientation::shown_edge(orientation, u, v);
    let turned = |p: (f64, f64)| (0..turns % 4).fold(p, |(x, y), _| (1.0 - y, x));
    let [l, t, r, b] = rect;
    let (a, c) = (turned(shown((l, t))), turned(shown((r, b))));
    [a.0.min(c.0), a.1.min(c.1), a.0.max(c.0), a.1.max(c.1)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upright_follows_the_decoders_transform() {
        // A 4×2 stored image whose only lit pixel block is the rectangle x 1..3, y 0..1; every orientation and turn
        // must put the rectangle where decode::orient (and then quarter turns) puts the pixels.
        let (w, h) = (4usize, 2usize);
        let lit = |x: usize, y: usize| (1..3).contains(&x) && y < 1;
        let pixels: Vec<u8> = (0..h).flat_map(|y| (0..w).map(move |x| u8::from(lit(x, y)))).collect();
        let rect = [1.0 / 4.0, 0.0, 3.0 / 4.0, 1.0 / 2.0];
        for orientation in 1..=8u8 {
            for turns in 0..4u8 {
                let (mut px, mut pw, mut ph) = pg_codec::decode::transform(&pixels, w, h, 1, orientation);
                for _ in 0..turns {
                    (px, pw, ph) = pg_codec::decode::transform(&px, pw, ph, 1, 6);
                }
                let [x0, y0, x1, y1] = upright(rect, orientation, turns);
                for y in 0..ph {
                    for x in 0..pw {
                        let (cx, cy) = ((x as f64 + 0.5) / pw as f64, (y as f64 + 0.5) / ph as f64);
                        let inside = (x0..x1).contains(&cx) && (y0..y1).contains(&cy);
                        assert_eq!(px[y * pw + x] == 1, inside, "orientation {orientation}, {turns} turns, ({x}, {y})");
                    }
                }
            }
        }
    }
}
