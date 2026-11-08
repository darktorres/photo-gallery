//! A photo re-encoded for a service that takes fewer formats than the app reads: a TIFF as a JPEG for the Yandex search
//! (`pg_app::yandex`). The pixels come from [`display::decode`], the one place that reads the EXIF orientation, so the
//! JPEG shows the photo upright, as the viewer does.

use anyhow::{Context, Result};
use turbojpeg::{Compressor, Image, PixelFormat, Subsamp};

use pg_codec::decode::{Decoder, MinSize};
use pg_model::turns::Turns;

use crate::display;

/// libjpeg-turbo q95 without chroma subsampling (4:4:4): a copy for a search, kept close to the photo, not a thumbnail.
const QUALITY: i32 = 95;

/// `bytes` (a file in any format the codec decodes) as a JPEG of the same size, upright. Fails when the file can't be
/// decoded or libjpeg-turbo refuses the pixels.
pub fn to_jpeg(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut decoder = Decoder::new()?;
    let decoded = display::decode(&mut decoder, bytes, MinSize::Full, Turns::NONE)?;
    let image = &decoded.image;
    let pitch = image.width.checked_mul(3).context("the image's row is longer than memory")?;
    let mut compressor = Compressor::new()?;
    compressor.set_quality(QUALITY)?;
    compressor.set_subsamp(Subsamp::None)?;
    let jpeg = compressor.compress_to_vec(Image {
        pixels: image.pixels.as_slice(),
        width: image.width,
        pitch,
        height: image.height,
        format: PixelFormat::RGB,
    })?;
    Ok(jpeg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pg_model::format::Format;

    #[test]
    fn a_tiff_becomes_a_jpeg_of_its_size_and_colour() {
        let (w, h) = (16u32, 8u32);
        let rgb: Vec<u8> = (0..w * h).flat_map(|_| [200u8, 100, 50]).collect();
        let tiff = pg_testkit::tiff_rgb(w, h, &rgb).unwrap();
        let jpeg = to_jpeg(&tiff).unwrap();
        assert_eq!(Format::sniff(&jpeg), Some(Format::Jpeg));
        let decoded = display::decode(&mut Decoder::new().unwrap(), &jpeg, MinSize::Full, Turns::NONE).unwrap();
        assert_eq!((decoded.image.width, decoded.image.height), (16, 8));
        // One flat colour at q95, 4:4:4: every pixel within a few levels of it.
        for (got, want) in decoded.image.pixels.iter().zip(rgb.iter()) {
            assert!(got.abs_diff(*want) <= 4, "{got} for {want}");
        }
    }

    #[test]
    fn bytes_that_are_no_image_are_an_error() {
        assert!(to_jpeg(b"not an image at all").is_err());
    }
}
