//! The border detector over a pipe, for the eval (`eval/imgload.py`, `embed.py --trim`): images in on stdin, each one's
//! content box out on stdout. No catalog, no home: the pixels come from the caller, which decodes them its own way.

use std::io::{BufRead, Write};

use anyhow::{Context, Result, ensure};
use pg_analysis::borders::{Detector, Params};
use pg_codec::pixels::Rgb;
use pg_model::detection;

/// The most pixels one image may have: the decoders' cap (`pg_codec::max_decode_bytes`) at three bytes a pixel. A
/// header above it is refused before any pixel is read, so the pipe can't ask for a 3 GB buffer.
fn max_pixels() -> usize {
    usize::try_from(pg_codec::max_decode_bytes() / 3).unwrap_or(usize::MAX)
}

/// Look for borders in images read from `input` until it ends: each one's width and height (u32, little-endian),
/// then its pixels (RGB, rows top to bottom). For each, a line on `output`: what is left without its borders, as
/// `left top right bottom` (right and bottom exclusive; the whole image when it has none, or is too small to look
/// at). Returns how many it read.
pub fn serve(mut input: impl BufRead, mut output: impl Write) -> Result<usize> {
    let mut n = 0;
    let mut finder = Detector::default();
    let mut img = Rgb { width: 0, height: 0, pixels: Vec::new() };
    loop {
        if input.fill_buf()?.is_empty() {
            return Ok(n);
        }
        let mut size = [[0u8; 4]; 2];
        input.read_exact(size.as_flattened_mut()).context("reading an image's size")?;
        let dims = size.map(u32::from_le_bytes);
        let [w, h] = dims.map(pg_model::num::index);
        ensure!(w.checked_mul(h).is_some_and(|p| p <= max_pixels()), "image {n}: {w} × {h} is too large");
        (img.width, img.height) = (w, h);
        img.pixels.resize(w * h * 3, 0);
        input.read_exact(&mut img.pixels).with_context(|| format!("reading image {n}'s pixels"))?;
        let [l, t, r, b] = if w.min(h) >= detection::MIN_SIDE {
            finder.detect(&img, &Params::default())?.content
        } else {
            let [w, h] = dims;
            [0, 0, w, h]
        };
        writeln!(output, "{l} {t} {r} {b}")?;
        output.flush()?;
        n += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(w: u32, h: u32, pixels: &[u8]) -> Vec<u8> {
        [&w.to_le_bytes()[..], &h.to_le_bytes(), pixels].concat()
    }

    #[test]
    fn piped_images_get_their_content_boxes() {
        let input =
            [frame(320, 240, &pg_testkit::letterboxed_rgb(320, 240, 40)), frame(32, 32, &[7; 32 * 32 * 3])].concat();
        let mut output = Vec::new();
        assert_eq!(serve(&input[..], &mut output).unwrap(), 2);
        let lines: Vec<Vec<u32>> = String::from_utf8(output)
            .unwrap()
            .lines()
            .map(|l| l.split(' ').map(|v| v.parse().unwrap()).collect())
            .collect();
        // The bars and the detector's pixel of margin past the seam; the tiny one isn't looked at.
        let [l, t, r, b] = lines[0][..] else { panic!("{lines:?}") };
        assert!(l == 0 && r == 320 && (40..43).contains(&t) && (197..=200).contains(&b), "{lines:?}");
        assert_eq!(lines[1], [0, 0, 32, 32]);
        // A stream cut short is an error, not a silent end.
        assert!(serve(&input[..20], &mut Vec::new()).is_err());
    }

    #[test]
    fn a_header_above_the_pixel_limit_is_refused_before_its_pixels() {
        // 20000 × 20000 is 400 M pixels, over `max_pixels`; no pixel bytes follow, so the refusal is the header's.
        let mut input = Vec::new();
        input.extend_from_slice(&20_000u32.to_le_bytes());
        input.extend_from_slice(&20_000u32.to_le_bytes());
        let err = serve(&input[..], &mut Vec::new()).unwrap_err();
        assert!(err.to_string().contains("too large"), "{err}");
    }
}
