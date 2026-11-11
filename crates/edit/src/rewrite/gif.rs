//! A still GIF: its one image decoded from LZW, turned or cropped, and encoded again, its other blocks kept.

use anyhow::{Context, Result, bail, ensure};
use pg_container::{GifBlock, GifImage, gif_blocks, gif_header_len, gif_sub_block_data, gif_sub_blocks};

use super::op::Op;
use super::raster::{decoded_size, zeroed};

/// The `pixels` color indices of a GIF image, from its LZW data with its minimum code size. The stream must end with
/// its end code. What it holds past the image's pixels (some encoders write more) is decoded into a small buffer and
/// dropped: the frame's size, not the stream, bounds what is kept, so a few KB of file that claim a 65535 x 65535 frame
/// make no buffer larger than the frame. An image with no pixels decodes nothing. `pixels` is a size
/// [`decoded_size`] passed; the buffer is allocated fallibly.
fn decode_indices(lzw: &[u8], min_code: u8, pixels: usize) -> Result<Vec<u8>> {
    let mut indices = zeroed(pixels)?;
    if pixels == 0 {
        return Ok(indices);
    }
    let mut past = [0u8; 4096];
    let mut decoder = weezl::decode::Decoder::new(weezl::BitOrder::Lsb, min_code);
    let (mut read, mut written) = (0, 0);
    loop {
        let input = lzw.get(read..).unwrap_or_default();
        let filling = written < pixels;
        let out = if filling { indices.get_mut(written..).unwrap_or_default() } else { past.as_mut_slice() };
        let step = decoder.decode_bytes(input, out);
        read += step.consumed_in;
        if filling {
            written += step.consumed_out;
        }
        match step.status.context("damaged GIF")? {
            weezl::LzwStatus::Done => break,
            weezl::LzwStatus::Ok => {}
            weezl::LzwStatus::NoProgress => bail!("damaged GIF: its image data has no end code"),
        }
    }
    ensure!(written == pixels, "GIF image data is short");
    Ok(indices)
}

/// A still GIF whose image has no part left on the screen after the turn or crop (no pixels, or wholly off the
/// screen): there is nothing to rewrite. `rotate` takes it as the catalog's, as it does an animation; `crop` reports
/// it.
#[derive(Debug)]
pub(crate) struct NothingLeft;

impl std::fmt::Display for NothingLeft {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("nothing of the GIF's image is left")
    }
}

impl std::error::Error for NothingLeft {}

/// A still GIF; `None` for an animation (more than one image). A still image with nothing left ([`NothingLeft`]).
pub(crate) fn rewrite(bytes: &[u8], op: Op) -> Result<Option<Vec<u8>>> {
    let mut blocks = gif_blocks(bytes);
    let (mut images, mut image) = (0, None);
    for b in blocks.by_ref() {
        if let GifBlock::Image { range, image: descriptor } = b {
            images += 1;
            image = Some((range, descriptor));
        }
    }
    if !blocks.complete() {
        bail!("damaged GIF");
    }
    let (1, Some((range, descriptor))) = (images, image) else { return Ok(None) };
    let header = gif_header_len(bytes).context("damaged GIF")?;
    let u16_at = |i: usize| {
        bytes
            .get(i..)
            .and_then(<[u8]>::first_chunk::<2>)
            .map(|&b| usize::from(u16::from_le_bytes(b)))
            .context("damaged GIF")
    };
    let (sw, sh) = (u16_at(6)?, u16_at(8)?);
    // The logical screen, as the app's decoder reserves it (RGBA): a GIF it can't show isn't rewritten.
    decoded_size("the GIF's screen", &[sw, sh, 4])?;
    let d = range.start;
    let GifImage { left, top, width, height, packed, code_size: min_code, .. } = descriptor;
    let (left, top, w, h) = (usize::from(left), usize::from(top), usize::from(width), usize::from(height));
    let code_at = d + descriptor.code_at();
    // The sizes the app's GIF decoder takes (the `gif` crate refuses others): a GIF the viewer can't show isn't
    // rewritten, and weezl's coders, which panic for a size above 12, are never given one.
    ensure!((1..=11).contains(&min_code), "damaged GIF: an LZW minimum code size of {min_code}");

    // The LZW stream, from its sub-blocks.
    let (lzw, _) = gif_sub_block_data(bytes, code_at + 1).context("damaged GIF")?;
    let mut indices = decode_indices(&lzw, min_code, decoded_size("the GIF's image", &[w, h])?)?;
    // (An image no pixel wide has no rows to reorder.)
    if descriptor.interlaced() && w > 0 {
        // Interlaced: rows stored in four passes. Each stored row is paired with its place, then put back in order.
        let order = (0..h).step_by(8).chain((4..h).step_by(8)).chain((2..h).step_by(4)).chain((1..h).step_by(2));
        let mut rows: Vec<(usize, &[u8])> = order.zip(indices.chunks_exact(w)).collect();
        rows.sort_unstable_by_key(|&(y, _)| y);
        indices = rows.into_iter().flat_map(|(_, row)| row).copied().collect();
    }
    // The frame's place on the logical screen changes with it.
    let ((nleft, ntop), kept, (nsw, nsh)) = op.frame([left, top, w, h], sw, sh).ok_or(NothingLeft)?;
    let (kept_pixels, kw, kh) = Op::Crop(kept).apply(&indices, w, h, 1)?;
    let (pixels, nw, nh) = match op {
        Op::Orient(_) => op.apply(&kept_pixels, kw, kh, 1)?,
        Op::Crop(_) => (kept_pixels, kw, kh),
    };
    let data = weezl::encode::Encoder::new(weezl::BitOrder::Lsb, min_code).encode(&pixels)?;

    let mut out = bytes.get(..header).context("damaged GIF")?.to_vec();
    let screen = out.first_chunk_mut::<10>().context("damaged GIF")?;
    screen[6..8].copy_from_slice(&u16::try_from(nsw)?.to_le_bytes());
    screen[8..10].copy_from_slice(&u16::try_from(nsh)?.to_le_bytes());
    out.extend_from_slice(bytes.get(header..d).context("damaged GIF")?);
    out.push(0x2C);
    for v in [nleft, ntop, nw, nh] {
        out.extend_from_slice(&u16::try_from(v)?.to_le_bytes());
    }
    out.push(packed & !0x40); // written non-interlaced
    out.extend_from_slice(bytes.get(d + 10..code_at + 1).context("damaged GIF")?); // local color table, LZW code size
    out.extend(gif_sub_blocks(&data));
    out.extend_from_slice(bytes.get(range.end..).context("damaged GIF")?);
    Ok(Some(out))
}

#[cfg(test)]
mod tests {
    use pg_model::orientation::Orientation;

    use super::*;

    /// The offset of the LZW minimum code size of `gif`'s one image: after its descriptor and local color table.
    fn min_code_at(gif: &[u8]) -> usize {
        gif_blocks(gif)
            .find_map(|b| match b {
                GifBlock::Image { range, image } => Some(range.start + image.code_at()),
                _ => None,
            })
            .unwrap()
    }

    /// A minimum code size the app's GIF decoder refuses is a damaged GIF, not a panic in weezl (above 12) or a
    /// rewrite of a file the viewer can't show (0 and 12).
    #[test]
    fn a_gif_with_a_bad_minimum_code_size_is_an_error() {
        let rgb: Vec<u8> = (0..4 * 3 * 3).map(|i| u8::try_from(i * 7 % 256).unwrap()).collect();
        let good = pg_testkit::gif_rgb(4, 3, &rgb).unwrap();
        let at = min_code_at(&good);
        assert!((1..=11).contains(&good[at]), "the test GIF's own size is {}", good[at]);
        for op in [Op::Orient(Orientation::new(6).unwrap()), Op::Crop([1, 1, 2, 1])] {
            assert!(rewrite(&good, op).unwrap().is_some(), "the test GIF itself is rewritten");
            for size in [0, 12, 13, 255] {
                let mut bad = good.clone();
                bad[at] = size;
                let error = rewrite(&bad, op).unwrap_err().to_string();
                assert!(error.contains("damaged GIF"), "size {size}: {error}");
            }
        }
    }

    /// The decoded image is held to the frame's size, and its stream must end with an end code.
    #[test]
    fn decoded_indices_are_held_to_the_frame_and_need_the_end_code() {
        let pixels: Vec<u8> = (0..64u8).map(|i| i % 4).collect();
        let lzw = weezl::encode::Encoder::new(weezl::BitOrder::Lsb, 2).encode(&pixels).unwrap();
        assert_eq!(decode_indices(&lzw, 2, 64).unwrap(), pixels);
        // A frame smaller than the stream: the stream's first pixels, the rest read and dropped.
        assert_eq!(decode_indices(&lzw, 2, 10).unwrap(), pixels[..10]);
        // A frame larger than the stream: short.
        assert!(decode_indices(&lzw, 2, 65).is_err());
        // No pixels: nothing decoded.
        assert!(decode_indices(&lzw, 2, 0).unwrap().is_empty());
        // Cut before its end code: an error, as weezl's own `decode` has it.
        assert!(decode_indices(&lzw[..lzw.len() - 2], 2, 10).is_err());
    }

    /// The one image of `gif`: where its descriptor starts, and its left, top, width and height.
    fn descriptor(gif: &[u8]) -> (usize, [usize; 4]) {
        let d = gif_blocks(gif)
            .find_map(|b| match b {
                GifBlock::Image { range, .. } => Some(range.start),
                _ => None,
            })
            .unwrap();
        let at = |i: usize| usize::from(u16::from_le_bytes([gif[d + i], gif[d + i + 1]]));
        (d, [at(1), at(3), at(5), at(7)])
    }

    /// A turned GIF frame is clipped to the logical screen, once, in `Op::frame`: a frame with no pixels or wholly
    /// past the screen keeps nothing (no underflow), and one that sticks out keeps the part on the screen.
    #[test]
    fn a_turned_gif_frame_is_clipped_to_the_screen() {
        let six = Op::Orient(Orientation::new(6).unwrap());
        // On the screen: all of it, placed by its turned corners.
        assert_eq!(six.frame([1, 0, 2, 3], 4, 3), Some(((0, 1), [0, 0, 2, 3], (3, 4))));
        // Sticking out past the right and bottom edges: the 2 x 2 on the screen.
        assert_eq!(six.frame([2, 1, 5, 5], 4, 3), Some(((0, 2), [0, 0, 2, 2], (3, 4))));
        // Past the screen's edge, or no pixels: nothing.
        assert_eq!(six.frame([5, 0, 2, 2], 4, 3), None);
        assert_eq!(six.frame([0, 3, 2, 2], 4, 3), None);
        assert_eq!(six.frame([1, 1, 0, 2], 4, 3), None);
        assert_eq!(six.frame([1, 1, 2, 0], 4, 3), None);
        // Every orientation: the kept part fits the screen, and so does its new place.
        for o in 1..=8 {
            let op = Op::Orient(Orientation::new(o).unwrap());
            let ((x, y), [_, _, kw, kh], (nsw, nsh)) = op.frame([2, 1, 5, 5], 4, 3).unwrap();
            let (nw, nh) = op.size(kw, kh);
            assert!(x + nw <= nsw && y + nh <= nsh, "orientation {o}");
        }

        // A whole file: a 4 x 3 frame on a 3 x 2 screen turns as its 3 x 2 part on the screen.
        let rgb: Vec<u8> = (0..4 * 3 * 3).map(|i| u8::try_from(i * 7 % 256).unwrap()).collect();
        let mut file = pg_testkit::gif_rgb(4, 3, &rgb).unwrap();
        file[6..10].copy_from_slice(&[3, 0, 2, 0]);
        let out = rewrite(&file, six).unwrap().unwrap();
        assert_eq!(&out[6..10], &[2, 0, 3, 0], "the turned screen");
        assert_eq!(descriptor(&out).1, [0, 0, 2, 3]);
        // A frame wholly past the screen: nothing to turn, a `NothingLeft` error (not a wrapped placement).
        let (d, _) = descriptor(&file);
        file[d + 1..d + 3].copy_from_slice(&10u16.to_le_bytes());
        assert!(rewrite(&file, six).unwrap_err().is::<NothingLeft>());
    }
}
