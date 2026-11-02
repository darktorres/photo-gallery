use crate::compat::*;
use crate::convolution::{Chunk, Chunks, Coefficients, Normalized};

// This code is based on C-implementation from Pillow-SIMD package for Python
// https://github.com/uploadcare/pillow-simd

const fn get_clip_table() -> [u8; 1280] {
    let mut table = [0u8; 1280];
    let mut i: usize = 640;
    while i < 640 + 255 {
        table[i] = (i - 640) as u8;
        i += 1;
    }
    while i < 1280 {
        table[i] = 255;
        i += 1;
    }
    table
}

// Handles values form -640 to 639.
static CLIP8_LOOKUPS: [u8; 1280] = get_clip_table();

// 8 bits for a result. Filter can have negative areas.
// In one case, the sum of the coefficients will be negative,
// in the other it will be more than 1.0. That is why we need
// two extra bits for overflow and i32 type.
const PRECISION_BITS: u8 = 32 - 8 - 2;
// We use i16 type to store coefficients.
const MAX_COEFFS_PRECISION: u8 = 16 - 1;

// photo-gallery: the chunks are views over the coefficients' normalized form, which the
// coefficients keep (`Normalized`), in one buffer, instead of a `Vec` per output pixel.
pub(crate) type CoefficientsI16Chunk<'a> = Chunk<'a, i16>;

pub(crate) struct Normalizer16<'a> {
    precision: u8,
    chunks: Chunks<'a, i16>,
}

/// The largest weight, and the most bits of precision with which its scaled value stays below
/// `1 << max_coeffs_precision`.
fn precision_of(values: &[f64], precision_bits: u8, fits: impl Fn(f64, u8) -> bool) -> u8 {
    let max_weight = values
        .iter()
        .max_by(|&x, &y| x.partial_cmp(y).unwrap())
        .unwrap_or(&0.0)
        .to_owned();

    let mut precision = 0u8;
    for cur_precision in 0..precision_bits {
        precision = cur_precision;
        if !fits(max_weight, precision + 1) {
            // The next value will be outside the range, so stop
            break;
        }
    }
    debug_assert!(precision >= 4); // required for some SIMD optimisations
    precision
}

/// Fills `normalized` from `coefficients` (if it isn't current), each used weight scaled by
/// `1 << precision` and converted by `convert`; the unused rest of each window is zero.
fn normalize<T: Copy + Default>(
    coefficients: &Coefficients,
    normalized: &mut Normalized<T>,
    precision: u8,
    convert: impl Fn(f64) -> T,
) {
    let window_size = coefficients.window_size;
    normalized.values.clear();
    if window_size > 0 {
        normalized
            .values
            .reserve(window_size * coefficients.bounds.len());
        let coef_chunks = coefficients.values.chunks_exact(window_size);
        for (chunk, bound) in coef_chunks.zip(&coefficients.bounds) {
            let size = bound.size as usize;
            normalized
                .values
                .extend(chunk.iter().take(size).map(|&v| convert(v)));
            normalized
                .values
                .extend(core::iter::repeat_n(T::default(), window_size - size));
        }
    }
    normalized.precision = precision;
    normalized.current = true;
}

impl<'a> Normalizer16<'a> {
    #[inline]
    pub fn new(coefficients: &'a mut Coefficients) -> Self {
        if !coefficients.i16.current {
            let precision = precision_of(&coefficients.values, PRECISION_BITS, |w, p| {
                let next_value: i32 = (w * (1 << p) as f64).round() as i32;
                next_value < (1 << MAX_COEFFS_PRECISION)
            });
            let scale = (1 << precision) as f64;
            let mut normalized = core::mem::take(&mut coefficients.i16);
            normalize(coefficients, &mut normalized, precision, |v| {
                (v * scale).round() as i16
            });
            coefficients.i16 = normalized;
        }
        let coefficients: &'a Coefficients = coefficients;
        Self {
            precision: coefficients.i16.precision,
            chunks: Chunks::new(&coefficients.i16.values, coefficients),
        }
    }

    #[inline(always)]
    pub fn precision(&self) -> u8 {
        self.precision
    }

    #[inline(always)]
    pub fn chunks(&self) -> Chunks<'a, i16> {
        self.chunks
    }

    pub fn chunks_len(&self) -> usize {
        self.chunks.len()
    }

    /// # Safety
    /// The function must be used with the `v`
    /// such that the expression `v >> self.precision`
    /// produces a result in the range `[-512, 511]`.    
    #[inline(always)]
    pub unsafe fn clip(&self, v: i32) -> u8 {
        let index = (640 + (v >> self.precision)) as usize;
        // index must be in range [(640-512)..(640+511)]
        debug_assert!((128..=1151).contains(&index));
        *CLIP8_LOOKUPS.get_unchecked(index)
    }
}

// 16 bits for a result. Filter can have negative areas.
// In one cases the sum of the coefficients will be negative,
// in the other it will be more than 1.0. That is why we need
// two extra bits for overflow and i64 type.
const PRECISION16_BITS: u8 = 64 - 16 - 2;
// We use i32 type to store coefficients.
const MAX_COEFFS_PRECISION16: u8 = 32 - 1;

pub(crate) type CoefficientsI32Chunk<'a> = Chunk<'a, i32>;

/// Converts `Vec<f64>` into `Vec<i32>`.
pub(crate) struct Normalizer32<'a> {
    precision: u8,
    chunks: Chunks<'a, i32>,
}

impl<'a> Normalizer32<'a> {
    #[inline]
    pub fn new(coefficients: &'a mut Coefficients) -> Self {
        if !coefficients.i32.current {
            let precision = precision_of(&coefficients.values, PRECISION16_BITS, |w, p| {
                let next_value: i64 = (w * (1i64 << p) as f64).round() as i64;
                // The next value will be outside the range, so just stop
                next_value < (1i64 << MAX_COEFFS_PRECISION16)
            });
            let scale = (1i64 << precision) as f64;
            let mut normalized = core::mem::take(&mut coefficients.i32);
            normalize(coefficients, &mut normalized, precision, |v| {
                (v * scale).round() as i32
            });
            coefficients.i32 = normalized;
        }
        let coefficients: &'a Coefficients = coefficients;
        Self {
            precision: coefficients.i32.precision,
            chunks: Chunks::new(&coefficients.i32.values, coefficients),
        }
    }

    #[inline]
    pub fn precision(&self) -> u8 {
        self.precision
    }

    #[inline(always)]
    pub fn chunks(&self) -> Chunks<'a, i32> {
        self.chunks
    }

    #[inline(always)]
    pub fn chunks_len(&self) -> usize {
        self.chunks.len()
    }

    #[inline(always)]
    pub fn clip(&self, v: i64) -> u16 {
        (v >> self.precision).min(u16::MAX as i64).max(0) as u16
    }
}

macro_rules! try_process_in_threads_h {
    {$op: ident($src_view: ident, $dst_view: ident, $offset: ident, $($arg: ident),+$(,)?);}  => {
        #[allow(unused_labels)]
        'block: {
            #[cfg(feature = "rayon")]
            {
                use crate::threading::split_h_two_images_for_threading;
                use rayon::prelude::*;

                if let Some(iter) = split_h_two_images_for_threading($src_view, $dst_view, $offset) {
                    iter.for_each(|(src, mut dst)| {
                        $op(&src, &mut dst, 0, $($arg),+);
                    });
                    break 'block;
                }
            }
            $op($src_view, $dst_view, $offset, $($arg),+);
        }
    };
}

macro_rules! try_process_in_threads_v {
    {$op: ident($src_view: ident, $dst_view: ident, $offset: ident, $($arg: ident),+$(,)?);}  => {
        #[allow(unused_labels)]
        'block: {
            #[cfg(feature = "rayon")]
            {
                use crate::threading::split_v_two_images_for_threading;
                use rayon::prelude::*;

                if let Some(iter) = split_v_two_images_for_threading($src_view, $dst_view, $offset) {
                    iter.for_each(|(src, mut dst)| {
                        $op(&src, &mut dst, 0, $($arg),+);
                    });
                    break 'block;
                }
            }
            $op($src_view, $dst_view, $offset, $($arg),+);
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::convolution::Bound;

    fn get_coefficients(value: f64) -> Coefficients {
        Coefficients {
            values: vec![value],
            window_size: 1,
            bounds: vec![Bound { start: 0, size: 1 }],
            ..Default::default()
        }
    }

    #[test]
    fn test_minimal_precision() {
        // required for some SIMD optimisations
        assert!(Normalizer16::new(&mut get_coefficients(0.0)).precision() >= 4);
        assert!(Normalizer16::new(&mut get_coefficients(2.0)).precision() >= 4);
        assert!(Normalizer32::new(&mut get_coefficients(0.0)).precision() >= 4);
        assert!(Normalizer32::new(&mut get_coefficients(2.0)).precision() >= 4);
    }
}
