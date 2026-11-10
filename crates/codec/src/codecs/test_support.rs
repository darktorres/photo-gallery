//! Helpers the conversion tests and the decode tests share.

use image::DynamicImage;

use super::convert::over_white;
use pg_model::num;

/// `img` as 8-bit RGB the way the decode was done before this module: `into_rgba8` and the flattening onto white,
/// or `into_rgb8`.
pub(super) fn flattened(img: DynamicImage) -> (usize, usize, Vec<u8>) {
    let (w, h) = (num::index(img.width()), num::index(img.height()));
    let px = if img.color().has_alpha() {
        img.into_rgba8().pixels().flat_map(|p| [0, 1, 2].map(|c| over_white(p[c], p[3]))).collect()
    } else {
        img.into_rgb8().into_raw()
    };
    (w, h, px)
}

/// `values`, each as its `N` bytes.
pub(super) fn ne_bytes<const N: usize, T: Copy>(values: &[T], to: impl Fn(T) -> [u8; N]) -> Vec<u8> {
    values.iter().flat_map(|&v| to(v)).collect()
}

/// `len` bytes of noise, different for each `seed`.
pub(super) fn noise(len: usize, seed: usize) -> Vec<u8> {
    // The low byte of each hash.
    (0..len).map(|i| ((i ^ seed).wrapping_mul(2654435761) >> 13).to_le_bytes()[0]).collect()
}
