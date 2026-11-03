//! The pixel buffers the codecs, the decoder and the workers share. Nothing here depends on a decoder, so the
//! codecs and the decoder can both use it (R-05).

/// An upright RGB image.
#[derive(Clone, Default)]
pub struct Rgb {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u8>,
}

/// Free `buf` if it holds more than `bytes` (after one image far larger than the rest), so a worker's buffers don't
/// keep the largest image's size for the rest of a pass.
pub fn release_over(buf: &mut Vec<u8>, bytes: usize) {
    if buf.capacity() > bytes {
        *buf = Vec::new();
    }
}
