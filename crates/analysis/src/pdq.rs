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
            rgb.as_chunks::<3>()
                .0
                .iter()
                .map(|p| f32::from(p[0]) * 0.299 + f32::from(p[1]) * 0.587 + f32::from(p[2]) * 0.114),
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
            // x < width and y < height, so a `luma` of `width` × `height` always has the sample.
            if let Some(&sample) = luma.get(y * width + x) {
                *v = sample;
            }
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
    let transpose = v & 4 != 0;
    let mut out = [0.0; DCT_OUT * DCT_OUT];
    for (oi, out_row) in out.as_chunks_mut::<DCT_OUT>().0.iter_mut().enumerate() {
        // Output row oi is row oi of `a`, or column oi when transposed.
        let (start, step) = if transpose { (oi, DCT_OUT) } else { (oi * DCT_OUT, 1) };
        for (oj, (o, &x)) in out_row.iter_mut().zip(a.iter().skip(start).step_by(step)).enumerate() {
            // i = vertical frequency index, j = horizontal (of `a`)
            let (i, j) = if transpose { (oj, oi) } else { (oi, oj) };
            let mut x = x;
            if v & 1 != 0 && j % 2 == 0 {
                x = -x;
            }
            if v & 2 != 0 && i % 2 == 0 {
                x = -x;
            }
            *o = x;
        }
    }
    out
}

fn dct_matrix() -> &'static [[f32; BUFFER]; DCT_OUT] {
    static MATRIX: OnceLock<[[f32; BUFFER]; DCT_OUT]> = OnceLock::new();
    MATRIX.get_or_init(|| {
        let scale = (2.0f64 / pg_model::num::count(BUFFER)).sqrt();
        std::array::from_fn(|i| {
            std::array::from_fn(|j| {
                let angle = std::f64::consts::PI / 2.0 / pg_model::num::count(BUFFER)
                    * pg_model::num::count(i + 1)
                    * pg_model::num::count(2 * j + 1);
                to_f32(scale * angle.cos())
            })
        })
    })
}

/// `x` rounded to the nearest `f32`.
fn to_f32(x: f64) -> f32 {
    pg_model::num::f64_to_f32(x.clamp(f64::from(f32::MIN), f64::from(f32::MAX)))
}

/// One pass of the box filter along a row or column (`stride` 1 or the row length), shrinking the window at
/// both ends. Same operation order as the reference, so results are bit-exact.
fn box_1d(input: &[f32], start: usize, output: &mut [f32], len: usize, stride: usize, window: usize) {
    let half = (window + 2) / 2;
    let phase1 = half - 1;
    // The `len` cells, in order: the window's leading edge, its trailing edge (`window` cells behind) and the
    // output (`phase1` behind the leading edge) each walk them once.
    let cells = input.iter().skip(start).step_by(stride).take(len);
    let mut lead = cells.clone();
    let mut lag = cells;
    let mut out = output.iter_mut().skip(start).step_by(stride).take(len);

    let mut sum = 0.0f32;
    let mut count = 0.0f32;
    for &x in lead.by_ref().take(phase1) {
        sum += x;
        count += 1.0;
    }
    for (&x, o) in lead.by_ref().take(window - phase1).zip(out.by_ref()) {
        sum += x;
        count += 1.0;
        *o = sum / count;
    }
    for ((&x, &l), o) in lead.zip(lag.by_ref()).zip(out.by_ref()) {
        sum += x;
        sum -= l;
        *o = sum / count;
    }
    // The last `phase1` outputs, as the window's trailing edge leaves.
    for (&l, o) in lag.zip(out) {
        sum -= l;
        count -= 1.0;
        *o = sum / count;
    }
}

/// `box_1d` down every column of a `width` × `height` image at once: a running sum per column, a row at a time,
/// so memory is read in order and the columns vectorize (one column at a time strides by the row length). Each
/// column gets `box_1d`'s operations in the same order, so the results are the same bits. `sum` holds the running
/// sums; its buffer is kept between calls.
fn box_columns(input: &[f32], output: &mut [f32], width: usize, height: usize, window: usize, sum: &mut Vec<f32>) {
    let half = (window + 2) / 2;
    let phase1 = half - 1;
    // The rows, in order: the window's leading edge, its trailing edge (`window` rows behind) and the output rows
    // (`phase1` behind the leading edge) each walk them once.
    let rows = input.chunks_exact(width).take(height);
    let mut lead = rows.clone();
    let mut trail = rows;
    let mut out_rows = output.chunks_exact_mut(width).take(height);
    sum.clear();
    sum.resize(width, 0.0);
    let mut count = 0.0f32;
    for row in lead.by_ref().take(phase1) {
        for (s, &x) in sum.iter_mut().zip(row) {
            *s += x;
        }
        count += 1.0;
    }
    for (row, out) in lead.by_ref().take(window - phase1).zip(out_rows.by_ref()) {
        count += 1.0;
        for ((s, &x), o) in sum.iter_mut().zip(row).zip(out) {
            *s += x;
            *o = *s / count;
        }
    }
    for ((row, lag_row), out) in lead.zip(trail.by_ref()).zip(out_rows.by_ref()) {
        for (((s, &x), &lag), o) in sum.iter_mut().zip(row).zip(lag_row).zip(out) {
            *s += x;
            *s -= lag;
            *o = *s / count;
        }
    }
    // The last `phase1` output rows, as the window's trailing edge leaves.
    for (lag_row, out) in trail.zip(out_rows) {
        count -= 1.0;
        for ((s, &lag), o) in sum.iter_mut().zip(lag_row).zip(out) {
            *s -= lag;
            *o = *s / count;
        }
    }
}

fn dct_64_to_16(input: &[[f32; BUFFER]; BUFFER]) -> [f32; DCT_OUT * DCT_OUT] {
    let d = dct_matrix();
    // mid[i][j] = Σ_k d[i][k] · input[k][j], each sum taken in order of k from 0.0 (a row of `input` at a time
    // across every j: the same additions, in the same order, as one j at a time).
    let mut mid = [[0.0f32; BUFFER]; DCT_OUT];
    for (d_row, mid_row) in d.iter().zip(&mut mid) {
        for (&dk, input_row) in d_row.iter().zip(input) {
            for (m, &x) in mid_row.iter_mut().zip(input_row) {
                *m += dk * x;
            }
        }
    }
    // out[i * DCT_OUT + j] = Σ_k mid[i][k] · d[j][k].
    let mut out = [0.0f32; DCT_OUT * DCT_OUT];
    for (mid_row, out_row) in mid.iter().zip(out.as_chunks_mut::<DCT_OUT>().0.iter_mut()) {
        for (d_row, o) in d.iter().zip(out_row) {
            let mut s = 0.0f32;
            for (&m, &dk) in mid_row.iter().zip(d_row) {
                s += m * dk;
            }
            *o = s;
        }
    }
    out
}

/// Meta's quality: the sum of quantized gradients of the 64×64 buffer, in integer arithmetic as the reference.
fn quality(buf: &[[f32; BUFFER]; BUFFER]) -> u8 {
    let mut sum: i64 = 0;
    let step = |u: f32, v: f32| i64::from(to_step((u - v) * 100.0 / 255.0).abs());
    for (above, below) in buf.iter().zip(buf.iter().skip(1)) {
        sum += above.iter().zip(below).map(|(&u, &v)| step(u, v)).sum::<i64>();
    }
    for row in buf {
        sum += row.iter().zip(row.iter().skip(1)).map(|(&u, &v)| step(u, v)).sum::<i64>();
    }
    u8::try_from((sum / 90).min(100)).unwrap_or(u8::MAX)
}

/// A quantized gradient: `x` rounded toward zero. Gradients of 8-bit luma are within ±100.
fn to_step(x: f32) -> i32 {
    pg_model::num::f64_to_i32(f64::from(x.clamp(f32::from(i16::MIN), f32::from(i16::MAX))))
}

/// Bit k is set when coefficient k is above the median (the 128th smallest, as Torben's median returns).
fn to_bits(dct: &[f32; DCT_OUT * DCT_OUT]) -> Hash {
    let mut values = *dct;
    // The value a sort by `total_cmp` puts there (values it calls equal are the same bits).
    let (_, &mut median, _) = values.select_nth_unstable_by((DCT_OUT * DCT_OUT).div_ceil(2) - 1, f32::total_cmp);
    let mut out = [0u8; HASH_BYTES];
    // Coefficients 8c..8c + 8 go to byte HASH_BYTES - 1 - c, coefficient k to its bit k % 8.
    for (byte, coeffs) in out.iter_mut().rev().zip(dct.as_chunks::<8>().0) {
        for (bit, &v) in coeffs.iter().enumerate() {
            if v > median {
                *byte |= 1 << bit;
            }
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
                let (fx, fy) = (f32::from(u16::try_from(x).unwrap()), f32::from(u16::try_from(y).unwrap()));
                v.push(channel((fx * 0.05).sin() * 60.0 + (fy * 0.11).cos() * 60.0 + 128.0));
                v.push(channel(((fx + fy) * 0.03).sin() * 100.0 + 128.0));
                v.push(channel((fx * 0.02).cos() * (fy * 0.04).sin() * 100.0 + 128.0));
            }
        }
        v
    }

    /// `x` rounded toward zero, as a channel value.
    fn channel(x: f32) -> u8 {
        pg_model::num::f64_to_u8(f64::from(x))
    }

    fn reference(rgb: &[u8], w: usize, h: usize) -> ([u8; 32], f32) {
        let img =
            pdqhash::image::RgbImage::from_raw(u32::try_from(w).unwrap(), u32::try_from(h).unwrap(), rgb.to_vec())
                .unwrap();
        pdqhash::generate_pdq_full_size(&pdqhash::image::DynamicImage::ImageRgb8(img))
    }

    /// `dct16` as it was before the column pass went across every column at once: `box_1d` one column at a time.
    fn dct16_one_column_at_a_time(rgb: &[u8], width: usize, height: usize) -> ([f32; DCT_OUT * DCT_OUT], u8) {
        let mut luma: Vec<f32> = rgb
            .as_chunks::<3>()
            .0
            .iter()
            .map(|p| f32::from(p[0]) * 0.299 + f32::from(p[1]) * 0.587 + f32::from(p[2]) * 0.114)
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
        sizes.extend(
            (0..40).map(|_| (1 + usize::try_from(next() % 900).unwrap(), 1 + usize::try_from(next() % 900).unwrap())),
        );
        // One hasher for every size, as a worker keeps it: what an image leaves in its buffers changes no bit of
        // the next one's.
        let mut hasher = Hasher::default();
        for (w, h) in sizes {
            // The low byte of each random value.
            let rgb: Vec<u8> = (0..w * h * 3).map(|_| next().to_le_bytes()[0]).collect();
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
