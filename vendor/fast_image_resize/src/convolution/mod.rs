pub use filters::*;

use crate::compat::*;
use crate::pixels::InnerPixel;
use crate::{CpuExtensions, ImageView, ImageViewMut};

#[macro_use]
mod macros;

mod filters;
#[macro_use]
mod optimisations;
mod u8x4;
mod vertical_u8;
cfg_if::cfg_if! {
    if #[cfg(not(feature = "only_u8x4"))] {
        mod u8x1;
        mod u8x2;
        mod u8x3;
        mod u16x1;
        mod u16x2;
        mod u16x3;
        mod u16x4;
        mod i32x1;
        mod f32x1;
        mod f32x2;
        mod f32x3;
        mod f32x4;
        mod vertical_u16;
        mod vertical_f32;
    }
}

pub(crate) trait Convolution: InnerPixel {
    // photo-gallery: the coefficients are borrowed, so that the resizer keeps them (and their
    // normalized forms) for the next call.
    fn horiz_convolution(
        src_view: &impl ImageView<Pixel = Self>,
        dst_view: &mut impl ImageViewMut<Pixel = Self>,
        offset: u32,
        coeffs: &mut Coefficients,
        cpu_extensions: CpuExtensions,
    );

    fn vert_convolution(
        src_view: &impl ImageView<Pixel = Self>,
        dst_view: &mut impl ImageViewMut<Pixel = Self>,
        offset: u32,
        coeffs: &mut Coefficients,
        cpu_extensions: CpuExtensions,
    );
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Bound {
    pub start: u32,
    pub size: u32,
}

// photo-gallery: the coefficients of one axis, kept from call to call. `values` holds
// `window_size` weights per output pixel, of which its bound's first `size` are used; the
// normalized forms (`Normalizer16`, `Normalizer32`) are kept beside them in the same layout,
// made when first asked for. The chunks of every form are views over these buffers, so a
// convolution allocates nothing for its coefficients.
#[derive(Debug, Clone, Default)]
pub(crate) struct Coefficients {
    pub values: Vec<f64>,
    pub window_size: usize,
    pub bounds: Vec<Bound>,
    /// Subtracted from each bound's start by the chunks: the second pass of a two-pass
    /// convolution reads a temporary image that starts at the first bound.
    pub shift: u32,
    pub(crate) i16: Normalized<i16>,
    pub(crate) i32: Normalized<i32>,
}

/// A normalized form of the coefficients, in their layout. `current`: made from the values.
#[derive(Debug, Clone, Default)]
pub(crate) struct Normalized<T> {
    pub precision: u8,
    pub values: Vec<T>,
    pub current: bool,
}

/// One output pixel's weights, and the first input pixel they apply to.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Chunk<'a, T> {
    pub start: u32,
    pub values: &'a [T],
}

impl<'a, T> Chunk<'a, T> {
    #[inline(always)]
    pub fn values(&self) -> &'a [T] {
        self.values
    }
}

/// The chunks of a form of the coefficients, one per output pixel.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Chunks<'a, T> {
    values: &'a [T],
    window_size: usize,
    bounds: &'a [Bound],
    shift: u32,
}

impl<'a, T> Chunks<'a, T> {
    pub(crate) fn new(values: &'a [T], coeffs: &'a Coefficients) -> Self {
        debug_assert_eq!(values.len(), coeffs.window_size * coeffs.bounds.len());
        Self {
            values,
            window_size: coeffs.window_size,
            bounds: &coeffs.bounds,
            shift: coeffs.shift,
        }
    }

    #[inline(always)]
    pub fn len(&self) -> usize {
        self.bounds.len()
    }

    #[inline(always)]
    pub fn iter(&self) -> ChunksIter<'a, T> {
        ChunksIter {
            values: self.values,
            window_size: self.window_size,
            bounds: self.bounds.iter(),
            shift: self.shift,
        }
    }
}

impl<'a, T> IntoIterator for Chunks<'a, T> {
    type Item = Chunk<'a, T>;
    type IntoIter = ChunksIter<'a, T>;

    #[inline(always)]
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a, T> IntoIterator for &Chunks<'a, T> {
    type Item = Chunk<'a, T>;
    type IntoIter = ChunksIter<'a, T>;

    #[inline(always)]
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ChunksIter<'a, T> {
    values: &'a [T],
    window_size: usize,
    bounds: core::slice::Iter<'a, Bound>,
    shift: u32,
}

impl<'a, T> Iterator for ChunksIter<'a, T> {
    type Item = Chunk<'a, T>;

    #[inline(always)]
    fn next(&mut self) -> Option<Self::Item> {
        let bound = self.bounds.next()?;
        let (window, rest) = self.values.split_at(self.window_size);
        self.values = rest;
        Some(Chunk {
            start: bound.start - self.shift,
            values: &window[..bound.size as usize],
        })
    }

    #[inline(always)]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.bounds.size_hint()
    }
}

impl<T> ExactSizeIterator for ChunksIter<'_, T> {}

impl Coefficients {
    pub fn get_chunks(&self) -> Chunks<'_, f64> {
        Chunks::new(&self.values, self)
    }
}

// photo-gallery: computed into `out`, its buffers reused; its normalized forms are made
// again when next asked for, and its shift is cleared.
#[allow(clippy::too_many_arguments)]
pub(crate) fn precompute_coefficients(
    out: &mut Coefficients,
    in_size: u32,
    in0: f64, // Left/top border for cropping
    in1: f64, // Right/bottom border for cropping
    out_size: u32,
    filter: fn(f64) -> f64,
    filter_support: f64,
    adaptive_kernel_size: bool,
) {
    out.values.clear();
    out.bounds.clear();
    out.window_size = 0;
    out.shift = 0;
    out.i16.current = false;
    out.i32.current = false;
    if in_size == 0 || out_size == 0 {
        return;
    }
    let scale = (in1 - in0) / out_size as f64;
    if scale <= 0. {
        return;
    }
    let filter_scale = if adaptive_kernel_size {
        scale.max(1.0)
    } else {
        1.0
    };

    // Determine filter radius size (length of resampling filter)
    let filter_radius = filter_support * filter_scale;
    // Maximum number of coeffs per out pixel
    let window_size = filter_radius.ceil() as usize * 2 + 1;
    // Optimization: replace division by filter_scale
    // with multiplication by recip_filter_scale
    let recip_filter_scale = 1.0 / filter_scale;

    let count_of_coeffs = window_size * out_size as usize;
    let Coefficients {
        values: coeffs,
        bounds,
        ..
    } = &mut *out;
    coeffs.reserve(count_of_coeffs);
    bounds.reserve(out_size as usize);

    for out_x in 0..out_size {
        // Find the point in the input image corresponding to the center
        // of the current pixel in the output image.
        let in_center = in0 + (out_x as f64 + 0.5) * scale;

        // x_min and x_max are slice bounds for the input pixels relevant
        // to the output pixel we are calculating. Pixel x is relevant
        // if and only if (x >= x_min) && (x < x_max).
        // Invariant: 0 <= x_min < x_max <= width
        let x_min = (in_center - filter_radius).floor().max(0.) as u32;
        let x_max = (in_center + filter_radius).ceil().min(in_size as f64) as u32;

        let cur_index = coeffs.len();
        let mut ww: f64 = 0.0;

        // Optimization for follow for-cycle:
        // (x + 0.5) - in_center => x - (in_center - 0.5) => x - center
        let center = in_center - 0.5;

        let mut bound_start = x_min;
        let mut bound_end = x_max;

        // Calculate the weight of each input pixel from the given x-range.
        for x in x_min..x_max {
            let w: f64 = filter((x as f64 - center) * recip_filter_scale);
            if x == bound_start && w == 0. {
                // Don't use zero coefficients at the start of the bound;
                bound_start += 1;
            } else {
                coeffs.push(w);
                ww += w;
            }
        }
        for &c in coeffs.iter().rev() {
            if bound_end <= bound_start || c != 0. {
                break;
            }
            // Don't use zero coefficients at the end of the bound;
            bound_end -= 1;
        }

        if ww != 0.0 {
            // Normalise values of weights.
            // The sum of weights must be equal to 1.0.
            coeffs[cur_index..].iter_mut().for_each(|w| *w /= ww);
        }
        // The remaining values should stay empty if they are used despite x_max.
        coeffs.resize(cur_index + window_size, 0.);
        bounds.push(Bound {
            start: bound_start,
            size: bound_end - bound_start,
        });
    }
    out.window_size = window_size;
}
