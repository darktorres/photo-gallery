//! A file's pixels replaced in its own format, with everything else in it kept: the turns and the crops of `pg_edit`
//! (`rotate` and `crop`). Only the pixel data and the fields that describe it (size, placement, pixel density) change;
//! every other chunk or block (ICC profile, EXIF, XMP, text, comments) is copied as it was. JPEG is transformed in
//! the DCT domain; PNG, BMP, still GIF and WebP are decoded and re-encoded losslessly (WebP as lossless VP8L).
//!
//! Each format is its own module ([`jpeg`], [`png`], [`webp`], [`gif`], [`bmp`]), with a `rewrite` of a file's
//! bytes under an [`op::Op`]; they share the decoded rasters (the private `raster` module) and the fallible buffers
//! (the private `buffers` module), and the PNG and WebP chunks are in [`chunk`].

pub mod bmp;
mod buffers;
pub mod chunk;
pub mod gif;
pub mod jpeg;
pub mod op;
pub mod png;
mod raster;
pub mod webp;

#[cfg(test)]
mod tests {
    use pg_model::orientation::Orientation;

    use super::chunk::png_chunk;
    use super::op::Op;
    use super::raster::decoded_size;
    use super::{gif, png};

    // A header that declares a huge image is an error before anything that size is allocated (an allocation
    // failure would abort the process): a PNG's height, and a GIF's logical screen.
    #[test]
    fn a_huge_declared_size_is_an_error() {
        let mut file = Vec::new();
        let mut e = ::png::Encoder::new(&mut file, 1, 1);
        e.set_color(::png::ColorType::Rgb);
        e.set_depth(::png::BitDepth::Eight);
        let mut writer = e.write_header().unwrap();
        writer.write_image_data(&[1, 2, 3]).unwrap();
        writer.finish().unwrap();
        // The IHDR chunk: 8 bytes in, its data from 16 (width, then height), 25 bytes in all.
        let mut ihdr: [u8; 13] = file[16..29].try_into().unwrap();
        ihdr[4..8].copy_from_slice(&0x7FFF_FFFFu32.to_be_bytes());
        let huge = [&file[..8], &png_chunk(b"IHDR", &ihdr).unwrap()[..], &file[33..]].concat();
        for op in [Op::Orient(Orientation::new(6).unwrap()), Op::Crop([0, 0, 1, 1])] {
            let error = format!("{:#}", png::rewrite(&huge, op).unwrap_err());
            assert!(error.contains("too large"), "{error}");
        }
        assert!(
            png::rewrite(&file, Op::Orient(Orientation::new(6).unwrap())).is_ok(),
            "the test PNG itself is rewritten"
        );

        let rgb: Vec<u8> = (0..4 * 3 * 3).map(|i| u8::try_from(i % 256).unwrap()).collect();
        let mut gif_file = crate::fixtures::gif_rgb(4, 3, &rgb).unwrap();
        gif_file[6..10].copy_from_slice(&[0xFF; 4]);
        for op in [Op::Orient(Orientation::new(6).unwrap()), Op::Crop([0, 0, 1, 1])] {
            let error = format!("{:#}", gif::rewrite(&gif_file, op).unwrap_err());
            assert!(error.contains("too large"), "{error}");
        }
        // The cap is the decoders' own: a size at it is kept, a byte over it is refused, whatever its value.
        let cap = usize::try_from(crate::max_decode_bytes()).unwrap();
        assert_eq!(decoded_size("x", &[cap]).unwrap(), cap);
        assert!(decoded_size("x", &[cap + 1]).is_err());
        assert!(decoded_size("x", &[usize::MAX, 2]).is_err());
        assert_eq!(decoded_size("x", &[3, 4, 2]).unwrap(), 24);
    }
}
