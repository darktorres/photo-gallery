//! PDQ, Meta's 256-bit perceptual hash (tier 1), with the 8 dihedral variants (rotations and mirrors).
//!
//! A port of Meta's reference `pdqhashing.cpp`, following the arithmetic of the `pdqhash` crate, which is
//! bit-exact with Meta's Java hasher (tested against it in `tests`). That crate has no dihedral variants, which
//! the query side needs (FINDINGS "PDQ": mirrors and 90° rotations are only found with the 8-variant query).
//!
//! The input is the image already fitted into 512 px (see `analyze`), as Meta's hasher recommends and as M0
//! evaluated it (`eval/pdq.py`).
// A library reports through its results; printing is for the command line and the app.
#![cfg_attr(not(test), deny(clippy::print_stdout, clippy::print_stderr))]

use std::sync::OnceLock;

use pg_model::pdq::{HASH_BYTES, Hash, Pdq};

const BUFFER: usize = 64;
const DCT_OUT: usize = 16;

/// The hasher's buffers (a luma plane, the filter's pass between, its column sums), kept by a worker from image to
/// image. The input is at most 512 px on a side (see the module comment), so they stay small.
#[derive(Default)]
pub struct Hasher {
    luma: Vec<f32>,
    temp: Vec<f32>,
    sum: Vec<f32>,
}

impl Hasher {
    /// The hash of the image as is, plus its quality.
    pub fn hash_rgb(&mut self, rgb: &[u8], width: usize, height: usize) -> Pdq {
        let (dct, quality) = self.dct16(rgb, width, height);
        Pdq { hash: to_bits(&dct), quality }
    }

    /// The 8 dihedral hashes: index 0 is the image as is; the others are the hashes of its rotations and mirrors,
    /// derived from the DCT coefficients (as Meta's `dihedral` hasher does) rather than by re-hashing pixels.
    pub fn hash_rgb_dihedral(&mut self, rgb: &[u8], width: usize, height: usize) -> (Pdq, [Hash; 8]) {
        let (dct, quality) = self.dct16(rgb, width, height);
        let mut variants = [Hash([0; HASH_BYTES]); 8];
        for (v, out) in variants.iter_mut().enumerate() {
            *out = to_bits(&dihedral(&dct, v));
        }
        (Pdq { hash: variants[0], quality }, variants)
    }

    /// Luma, then a two-pass Jarosz tent filter, a 64×64 decimation and a 16×16 DCT, plus the quality metric.
    fn dct16(&mut self, rgb: &[u8], width: usize, height: usize) -> ([f32; DCT_OUT * DCT_OUT], u8) {
        assert!(width > 0 && height > 0 && rgb.len() == width * height * 3);
        let Hasher { luma, temp, sum } = self;
        luma.clear();
        luma.extend(
            rgb.as_chunks::<3>().0.iter().map(|p| p[0] as f32 * 0.299 + p[1] as f32 * 0.587 + p[2] as f32 * 0.114),
        );

        let window_cols = width.div_ceil(2 * BUFFER);
        let window_rows = height.div_ceil(2 * BUFFER);
        // Zeroed as a new buffer would be, so nothing of the image before can show through.
        temp.clear();
        temp.resize(luma.len(), 0.0);
        for _ in 0..2 {
            for r in 0..height {
                box_1d(luma, r * width, temp, width, 1, window_cols);
            }
            box_columns(temp, luma, width, height, window_rows, sum);
        }
        decimate(luma, width, height)
    }
}

/// The filtered luma sampled at the centers of a 64×64 grid: its 16×16 DCT and its quality.
fn decimate(luma: &[f32], width: usize, height: usize) -> ([f32; DCT_OUT * DCT_OUT], u8) {
    let mut buf = [[0.0f32; BUFFER]; BUFFER];
    for (i, row) in buf.iter_mut().enumerate() {
        let y = ((i * 2 + 1) * height) / (BUFFER * 2);
        for (j, v) in row.iter_mut().enumerate() {
            let x = ((j * 2 + 1) * width) / (BUFFER * 2);
            *v = luma[y * width + x];
        }
    }
    (dct_64_to_16(&buf), quality(&buf))
}

/// Variant `v` of the 16×16 DCT block: bit 0 mirrors left-right, bit 1 mirrors top-bottom, bit 2 transposes.
/// Together these give all 8 rotations and mirrors.
///
/// Meta's DCT rows are frequencies 1..=16 (the DC term is skipped), and mirroring an axis multiplies the
/// coefficient of frequency k by (−1)^k, so the even *indices* (odd frequencies) change sign.
fn dihedral(a: &[f32; DCT_OUT * DCT_OUT], v: usize) -> [f32; DCT_OUT * DCT_OUT] {
    let mut out = [0.0; DCT_OUT * DCT_OUT];
    for i in 0..DCT_OUT {
        // i = vertical frequency index, j = horizontal
        for j in 0..DCT_OUT {
            let mut x = a[i * DCT_OUT + j];
            if v & 1 != 0 && j % 2 == 0 {
                x = -x;
            }
            if v & 2 != 0 && i % 2 == 0 {
                x = -x;
            }
            let (oi, oj) = if v & 4 != 0 { (j, i) } else { (i, j) };
            out[oi * DCT_OUT + oj] = x;
        }
    }
    out
}

fn dct_matrix() -> &'static [[f32; BUFFER]; DCT_OUT] {
    static MATRIX: OnceLock<[[f32; BUFFER]; DCT_OUT]> = OnceLock::new();
    MATRIX.get_or_init(|| {
        let scale = (2.0f64 / BUFFER as f64).sqrt();
        std::array::from_fn(|i| {
            std::array::from_fn(|j| {
                let angle = std::f64::consts::PI / 2.0 / BUFFER as f64 * (i + 1) as f64 * (2 * j + 1) as f64;
                (scale * angle.cos()) as f32
            })
        })
    })
}

/// One pass of the box filter along a row or column (`stride` 1 or the row length), shrinking the window at
/// both ends. Same operation order as the reference, so results are bit-exact.
fn box_1d(input: &[f32], start: usize, output: &mut [f32], len: usize, stride: usize, window: usize) {
    let half = (window + 2) / 2;
    let phase1 = half - 1;
    let phase2 = window - half + 1;
    let out_off = phase1 * stride;
    let lag_off = phase2 * stride;

    let mut sum = 0.0f32;
    let mut count = 0.0f32;
    let phase1_end = start + out_off;
    for ri in (start..phase1_end).step_by(stride) {
        sum += input[ri];
        count += 1.0;
    }
    let phase2_end = start + window * stride;
    for ri in (phase1_end..phase2_end).step_by(stride) {
        sum += input[ri];
        count += 1.0;
        output[ri - out_off] = sum / count;
    }
    let phase3_end = start + len * stride;
    for ri in (phase2_end..phase3_end).step_by(stride) {
        let oi = ri - out_off;
        sum += input[ri];
        sum -= input[oi - lag_off];
        output[oi] = sum / count;
    }
    for oi in (start + (len - half + 1) * stride..phase3_end).step_by(stride) {
        sum -= input[oi - lag_off];
        count -= 1.0;
        output[oi] = sum / count;
    }
}

/// `box_1d` down every column of a `width` × `height` image at once: a running sum per column, a row at a time,
/// so memory is read in order and the columns vectorize (one column at a time strides by the row length). Each
/// column gets `box_1d`'s operations in the same order, so the results are the same bits. `sum` holds the running
/// sums; its buffer is kept between calls.
fn box_columns(input: &[f32], output: &mut [f32], width: usize, height: usize, window: usize, sum: &mut Vec<f32>) {
    let half = (window + 2) / 2;
    let phase1 = half - 1;
    let phase2 = window - half + 1;
    let row = |r: usize| &input[r * width..(r + 1) * width];
    sum.clear();
    sum.resize(width, 0.0);
    let mut count = 0.0f32;
    for r in 0..phase1 {
        for (s, &x) in sum.iter_mut().zip(row(r)) {
            *s += x;
        }
        count += 1.0;
    }
    for r in phase1..window {
        count += 1.0;
        let out = &mut output[(r - phase1) * width..][..width];
        for ((s, &x), o) in sum.iter_mut().zip(row(r)).zip(out) {
            *s += x;
            *o = *s / count;
        }
    }
    for r in window..height {
        let oi = r - phase1;
        let out = &mut output[oi * width..][..width];
        for (((s, &x), &lag), o) in sum.iter_mut().zip(row(r)).zip(row(oi - phase2)).zip(out) {
            *s += x;
            *s -= lag;
            *o = *s / count;
        }
    }
    for oi in height - half + 1..height {
        count -= 1.0;
        let out = &mut output[oi * width..][..width];
        for ((s, &lag), o) in sum.iter_mut().zip(row(oi - phase2)).zip(out) {
            *s -= lag;
            *o = *s / count;
        }
    }
}

fn dct_64_to_16(input: &[[f32; BUFFER]; BUFFER]) -> [f32; DCT_OUT * DCT_OUT] {
    let d = dct_matrix();
    let mut mid = [[0.0f32; BUFFER]; DCT_OUT];
    for i in 0..DCT_OUT {
        for j in 0..BUFFER {
            let mut s = 0.0f32;
            for k in 0..BUFFER {
                s += d[i][k] * input[k][j];
            }
            mid[i][j] = s;
        }
    }
    let mut out = [0.0f32; DCT_OUT * DCT_OUT];
    for i in 0..DCT_OUT {
        for j in 0..DCT_OUT {
            let mut s = 0.0f32;
            for k in 0..BUFFER {
                s += mid[i][k] * d[j][k];
            }
            out[i * DCT_OUT + j] = s;
        }
    }
    out
}

/// Meta's quality: the sum of quantized gradients of the 64×64 buffer, in integer arithmetic as the reference.
fn quality(buf: &[[f32; BUFFER]; BUFFER]) -> u8 {
    let mut sum: i64 = 0;
    let step = |u: f32, v: f32| (((u - v) * 100.0 / 255.0) as i32).abs() as i64;
    for pair in buf.windows(2) {
        sum += pair[0].iter().zip(&pair[1]).map(|(&u, &v)| step(u, v)).sum::<i64>();
    }
    for row in buf {
        sum += row.windows(2).map(|w| step(w[0], w[1])).sum::<i64>();
    }
    (sum / 90).min(100) as u8
}

/// Bit k is set when coefficient k is above the median (the 128th smallest, as Torben's median returns).
fn to_bits(dct: &[f32; DCT_OUT * DCT_OUT]) -> Hash {
    let mut sorted = *dct;
    sorted.sort_unstable_by(f32::total_cmp);
    let median = sorted[sorted.len().div_ceil(2) - 1];
    let mut out = [0u8; HASH_BYTES];
    for (k, &v) in dct.iter().enumerate() {
        if v > median {
            out[HASH_BYTES - 1 - k / 8] |= 1 << (k % 8);
        }
    }
    Hash(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_image(w: usize, h: usize) -> Vec<u8> {
        // Deterministic smooth texture with structure at several scales (a sharp sawtooth would alias on PDQ's
        // 64×64 sampling grid, which isn't mirror-symmetric, and move mirrored hashes by ~30 bits).
        let mut v = Vec::with_capacity(w * h * 3);
        for y in 0..h {
            for x in 0..w {
                let (fx, fy) = (x as f32, y as f32);
                v.push(((fx * 0.05).sin() * 60.0 + (fy * 0.11).cos() * 60.0 + 128.0) as u8);
                v.push((((fx + fy) * 0.03).sin() * 100.0 + 128.0) as u8);
                v.push(((fx * 0.02).cos() * (fy * 0.04).sin() * 100.0 + 128.0) as u8);
            }
        }
        v
    }

    fn reference(rgb: &[u8], w: usize, h: usize) -> ([u8; 32], f32) {
        let img = pdqhash::image::RgbImage::from_raw(w as u32, h as u32, rgb.to_vec()).unwrap();
        pdqhash::generate_pdq_full_size(&pdqhash::image::DynamicImage::ImageRgb8(img))
    }

    /// `dct16` as it was before the column pass went across every column at once: `box_1d` one column at a time.
    fn dct16_one_column_at_a_time(rgb: &[u8], width: usize, height: usize) -> ([f32; DCT_OUT * DCT_OUT], u8) {
        let mut luma: Vec<f32> = rgb
            .as_chunks::<3>()
            .0
            .iter()
            .map(|p| p[0] as f32 * 0.299 + p[1] as f32 * 0.587 + p[2] as f32 * 0.114)
            .collect();
        let window_cols = width.div_ceil(2 * BUFFER);
        let window_rows = height.div_ceil(2 * BUFFER);
        let mut temp = vec![0.0f32; luma.len()];
        for _ in 0..2 {
            for r in 0..height {
                box_1d(&luma, r * width, &mut temp, width, 1, window_cols);
            }
            for c in 0..width {
                box_1d(&temp, c, &mut luma, height, width, window_rows);
            }
        }
        decimate(&luma, width, height)
    }

    /// The same bits as before, every coefficient and the quality, on noise (every pixel different) at the sizes
    /// where the windows' phases meet the edges, and at random sizes.
    #[test]
    fn columns_at_once_change_no_bit() {
        let mut x: u64 = 0x2545_F491_4F6C_DD1D;
        let mut next = move || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        let mut sizes = vec![(1, 1), (1, 300), (300, 1), (127, 128), (128, 129), (129, 127), (512, 407), (97, 211)];
        sizes.extend((0..40).map(|_| (1 + (next() % 900) as usize, 1 + (next() % 900) as usize)));
        // One hasher for every size, as a worker keeps it: what an image leaves in its buffers changes no bit of
        // the next one's.
        let mut hasher = Hasher::default();
        for (w, h) in sizes {
            let rgb: Vec<u8> = (0..w * h * 3).map(|_| next() as u8).collect();
            let (ours, q) = hasher.dct16(&rgb, w, h);
            let (before, q_before) = dct16_one_column_at_a_time(&rgb, w, h);
            assert_eq!(ours.map(f32::to_bits), before.map(f32::to_bits), "{w}x{h}");
            assert_eq!(q, q_before, "{w}x{h}");
        }
    }

    #[test]
    fn matches_reference_crate() {
        for (w, h) in [(512, 384), (300, 512), (64, 64), (97, 211)] {
            let rgb = test_image(w, h);
            let ours = Hasher::default().hash_rgb(&rgb, w, h);
            let (theirs, _) = reference(&rgb, w, h);
            assert_eq!(ours.hash.0, theirs, "{w}x{h}");
        }
    }

    /// Each derived variant must be close to the hash of the actually transformed pixels (not exact: the
    /// decimation grid isn't perfectly symmetric).
    #[test]
    fn dihedral_matches_transformed_pixels() {
        let (w, h) = (320, 240);
        let rgb = test_image(w, h);
        let mut hasher = Hasher::default();
        let (_, variants) = hasher.hash_rgb_dihedral(&rgb, w, h);
        for (v, variant) in variants.iter().enumerate() {
            let transpose = v & 4 != 0;
            let (ow, oh) = if transpose { (h, w) } else { (w, h) };
            let mut out = vec![0u8; rgb.len()];
            for oy in 0..oh {
                for ox in 0..ow {
                    let (mut x, mut y) = if transpose { (oy, ox) } else { (ox, oy) };
                    // The mirrors apply to the source axes before transposing.
                    if v & 1 != 0 {
                        x = w - 1 - x;
                    }
                    if v & 2 != 0 {
                        y = h - 1 - y;
                    }
                    let (s, d) = ((y * w + x) * 3, (oy * ow + ox) * 3);
                    out[d..d + 3].copy_from_slice(&rgb[s..s + 3]);
                }
            }
            let direct = hasher.hash_rgb(&out, ow, oh).hash;
            let d = direct.distance(variant);
            assert!(d <= 12, "variant {v}: distance {d}");
        }
    }
}
