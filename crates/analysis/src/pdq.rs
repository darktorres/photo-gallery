//! PDQ, Meta's 256-bit perceptual hash (tier 1), with the 8 dihedral variants (rotations and mirrors).
//!
//! A port of Meta's reference `pdqhashing.cpp`. On the smooth images of the recorded vectors (in `tests`) its hash
//! bits agree with that reference; that is not a claim about every input, nor about the DCT matrix, which is computed
//! here. The reference hashes one orientation only, and the query side needs the rotations and mirrors too: they are
//! only found with the 8-variant query.
//!
//! The input is the image already fitted into 512 px (see `analyze`), as Meta's hasher recommends and as the evaluation
//! in `eval/pdq.py` did.
use std::sync::OnceLock;

use pg_model::pdq::{HASH_BYTES, Hash, ORIENTATIONS, Pdq};

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
    /// The hash of the image as is, plus its quality: what [`Hasher::hash_rgb_dihedral`]'s first orientation is checked
    /// against.
    #[cfg(test)]
    pub fn hash_rgb(&mut self, rgb: &[u8], width: usize, height: usize) -> (Hash, u8) {
        let (dct, quality) = self.dct16(rgb, width, height);
        (to_bits(&dct), quality)
    }

    /// The picture's PDQ in its kept orientations (`pg_model::pdq::ORIENTATIONS`): index 0 is the image as is, 1..4 its
    /// mirrors, derived from the DCT coefficients (as Meta's `dihedral` hasher does) rather than by re-hashing pixels.
    /// The other four dihedral variants (4..8, the transposed ones) are the transposes of these, bit for bit
    /// (`Hash::transpose`), so they are not made.
    pub fn hash_rgb_dihedral(&mut self, rgb: &[u8], width: usize, height: usize) -> Pdq {
        let (dct, quality) = self.dct16(rgb, width, height);
        let mut orientations = [Hash([0; HASH_BYTES]); ORIENTATIONS];
        for (v, out) in orientations.iter_mut().enumerate() {
            *out = to_bits(&dihedral(&dct, v));
        }
        Pdq { orientations, quality }
    }

    /// All eight dihedral variants, the transposed ones made from their own coefficients: what the tests check the
    /// kept four and their transposes against.
    #[cfg(test)]
    fn all_variants(&mut self, rgb: &[u8], width: usize, height: usize) -> [Hash; 8] {
        let (dct, _) = self.dct16(rgb, width, height);
        std::array::from_fn(|v| to_bits(&dihedral(&dct, v)))
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

    /// A deterministic smooth texture for the properties that compare a hash with itself (the dihedral and kept-variant
    /// tests): the recorded vectors use `smooth_image`, whose bytes do not depend on the platform's libm.
    fn test_image(w: usize, h: usize) -> Vec<u8> {
        // Structure at several scales (a sharp sawtooth would alias on PDQ's 64×64 sampling grid, which isn't
        // mirror-symmetric, and move mirrored hashes by ~30 bits).
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

    /// A smooth test image in integer arithmetic only: three triangle waves of long period, one per channel. Smooth at
    /// every scale PDQ samples (a sharp sawtooth would alias on its 64×64 grid, which isn't mirror-symmetric, and move
    /// mirrored hashes by ~30 bits), and the same bytes on every platform (no `sin` or `cos`, whose last bit depends on
    /// the platform's libm).
    fn smooth_image(w: usize, h: usize) -> Vec<u8> {
        // A triangle wave of `period` steps, scaled to 0..=255.
        fn wave(v: usize, period: usize) -> u8 {
            let p = v % period;
            u8::try_from(p.min(period - p) * 255 / (period / 2)).unwrap()
        }
        let mut out = Vec::with_capacity(w * h * 3);
        for y in 0..h {
            for x in 0..w {
                out.push(wave(x + 2 * y, 512));
                out.push(wave(2 * x + y + 100, 700));
                out.push(wave(x + 3 * y + 37, 900));
            }
        }
        out
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

    // The same bits as before, every coefficient and the quality, on noise (every pixel different) at the sizes
    // where the windows' phases meet the edges, and at random sizes.
    #[test]
    fn columns_at_once_change_no_bit() {
        let mut rng = pg_testkit::Rng::new(0x2545_F491_4F6C_DD1D);
        let mut next = || rng.next_u64();
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

    /// The hashes of Meta's reference (`pdqhashing.cpp`, read through the `pdqhash` crate when these were recorded, and
    /// no longer a dependency), of `smooth_image` at each size. The hash bits agree with the reference on these images;
    /// the dihedral and noise tests cover the rest. Every vector is bit-exact: no tolerance.
    const REFERENCE_VECTORS: &[(usize, usize, [u8; 32])] = &[
        (
            512,
            384,
            [
                252, 232, 41, 220, 163, 200, 126, 158, 225, 232, 126, 248, 245, 232, 42, 152, 64, 182, 126, 230, 192,
                188, 144, 84, 20, 252, 213, 172, 171, 41, 1, 3,
            ],
        ),
        (
            300,
            512,
            [
                61, 172, 18, 218, 63, 172, 115, 250, 221, 82, 80, 134, 197, 137, 12, 222, 213, 168, 239, 30, 194, 76,
                77, 94, 250, 244, 95, 100, 85, 84, 0, 1,
            ],
        ),
        (
            64,
            64,
            [
                162, 186, 200, 80, 130, 10, 168, 242, 130, 165, 221, 95, 215, 247, 136, 0, 149, 85, 63, 175, 109, 117,
                169, 5, 83, 59, 168, 109, 115, 248, 169, 82,
            ],
        ),
        (
            97,
            211,
            [
                157, 59, 33, 210, 74, 7, 82, 236, 198, 41, 61, 224, 171, 23, 186, 244, 162, 157, 124, 33, 85, 207, 85,
                88, 1, 85, 174, 173, 170, 171, 170, 170,
            ],
        ),
        (
            128,
            96,
            [
                173, 153, 154, 102, 100, 185, 139, 102, 38, 153, 145, 110, 42, 217, 139, 166, 38, 89, 139, 182, 174,
                201, 107, 54, 218, 201, 116, 162, 85, 41, 38, 202,
            ],
        ),
        (
            256,
            256,
            [
                68, 94, 255, 149, 4, 38, 212, 111, 12, 172, 223, 225, 68, 76, 220, 91, 172, 134, 10, 237, 252, 130, 33,
                71, 187, 214, 33, 143, 85, 94, 85, 84,
            ],
        ),
        (
            200,
            150,
            [
                252, 38, 233, 212, 171, 22, 234, 116, 138, 198, 253, 144, 1, 38, 253, 104, 77, 10, 249, 90, 207, 182,
                24, 38, 243, 172, 6, 48, 242, 231, 138, 140,
            ],
        ),
        (
            33,
            17,
            [
                40, 29, 6, 78, 103, 96, 16, 76, 217, 233, 250, 207, 255, 227, 176, 12, 168, 168, 250, 94, 138, 34, 112,
                95, 200, 170, 147, 190, 182, 99, 156, 198,
            ],
        ),
        (
            640,
            480,
            [
                143, 50, 218, 252, 239, 128, 251, 252, 255, 128, 85, 60, 168, 112, 211, 44, 234, 112, 2, 252, 73, 80,
                83, 252, 86, 248, 172, 11, 85, 82, 8, 7,
            ],
        ),
        (
            1024,
            768,
            [
                170, 132, 5, 122, 253, 108, 77, 90, 253, 108, 69, 82, 171, 236, 221, 82, 175, 236, 253, 82, 85, 100, 0,
                13, 85, 99, 85, 74, 0, 31, 170, 181,
            ],
        ),
        (
            100,
            100,
            [
                42, 93, 212, 162, 41, 89, 218, 182, 117, 73, 170, 150, 165, 41, 82, 86, 165, 173, 106, 210, 85, 173,
                169, 90, 90, 165, 85, 106, 85, 85, 170, 170,
            ],
        ),
        (
            160,
            120,
            [
                233, 115, 2, 2, 84, 251, 81, 136, 202, 119, 42, 200, 78, 51, 113, 104, 78, 91, 123, 244, 202, 47, 155,
                216, 54, 83, 132, 220, 177, 147, 174, 166,
            ],
        ),
        (
            512,
            512,
            [
                234, 186, 57, 236, 254, 250, 125, 200, 234, 152, 32, 188, 100, 66, 56, 188, 106, 162, 193, 188, 158,
                122, 85, 252, 149, 168, 170, 41, 157, 169, 0, 3,
            ],
        ),
        (
            77,
            300,
            [
                255, 9, 128, 123, 234, 209, 170, 159, 117, 124, 85, 65, 64, 27, 255, 241, 170, 167, 170, 172, 85, 80,
                85, 87, 213, 84, 64, 1, 170, 170, 170, 170,
            ],
        ),
    ];

    #[test]
    fn matches_reference_vectors() {
        for (w, h, expected) in REFERENCE_VECTORS {
            let rgb = smooth_image(*w, *h);
            let (ours, _) = Hasher::default().hash_rgb(&rgb, *w, *h);
            assert_eq!(ours.0, *expected, "{w}x{h}");
        }
    }

    // Each derived variant must be close to the hash of the actually transformed pixels (not exact: the
    // decimation grid isn't perfectly symmetric).
    #[test]
    fn dihedral_matches_transformed_pixels() {
        let (w, h) = (320, 240);
        let rgb = test_image(w, h);
        let mut hasher = Hasher::default();
        let variants = hasher.all_variants(&rgb, w, h);
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
            let (direct, _) = hasher.hash_rgb(&out, ow, oh);
            let d = direct.distance(variant);
            assert!(d <= 12, "variant {v}: distance {d}");
        }
    }

    // The kept orientations are the first four variants, and the other four are their transposes bit for bit: what
    // the catalog (`analysis.pdq_dihedral` holds four) and the join (it transposes the queries) rely on.
    #[test]
    fn the_kept_orientations_and_their_transposes_are_the_eight_variants() {
        for (w, h) in [(512, 384), (300, 512), (64, 64), (97, 211), (320, 240)] {
            let rgb = test_image(w, h);
            let mut hasher = Hasher::default();
            let all = hasher.all_variants(&rgb, w, h);
            let pdq = hasher.hash_rgb_dihedral(&rgb, w, h);
            let (hash, quality) = hasher.hash_rgb(&rgb, w, h);
            assert_eq!(pdq.hash(), hash, "{w}x{h}");
            assert_eq!(pdq.quality, quality, "{w}x{h}");
            for v in 0..ORIENTATIONS {
                assert_eq!(pdq.orientations[v], all[v], "{w}x{h} variant {v}");
                assert_eq!(pdq.orientations[v].transpose(), all[v + 4], "{w}x{h} variant {}", v + 4);
            }
        }
    }
}
