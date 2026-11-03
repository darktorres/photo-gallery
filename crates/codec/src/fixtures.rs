//! Test fixtures for the crates that test their files: a TIFF the tiff crate's encoder writes (BigTIFF, which `image`
//! doesn't write). Built with the `fixtures` feature, which the crates' dev-dependencies enable.

/// An RGB8 TIFF written by the tiff crate's encoder, classic or BigTIFF (`image` writes only classic).
pub fn tiff_rgb8(w: u32, h: u32, pixels: &[u8], big: bool) -> Vec<u8> {
    use tiff::encoder::{TiffEncoder, colortype::RGB8};
    let mut out = std::io::Cursor::new(Vec::new());
    if big {
        TiffEncoder::new_big(&mut out).unwrap().write_image::<RGB8>(w, h, pixels).unwrap();
    } else {
        TiffEncoder::new(&mut out).unwrap().write_image::<RGB8>(w, h, pixels).unwrap();
    }
    out.into_inner()
}
