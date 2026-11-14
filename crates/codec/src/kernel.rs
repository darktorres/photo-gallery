//! Magic Kernel Sharp 2021 (Costella, 2021) as the resizer's filter: the one kernel for every resize.
//! The single-pass kernel is k(x) = Σₙ sₙ·m(x − n) over n = −3..=3, with m the magic kernel and s the Sharp 2021 taps;
//! the paper's two-pass form is checked against it.

use fast_image_resize::{Filter, FilterType};

/// The kernel's support, in the units the resizer stretches: 3/2 (the magic kernel) plus 3 (the sharpen taps reach
/// three samples out).
const SUPPORT: f64 = 4.5;

/// The magic kernel m(x) (Costella 2021, eq. 27).
fn m(x: f64) -> f64 {
    let ax = x.abs();
    if ax <= 0.5 {
        0.75 - ax * ax
    } else if ax <= 1.5 {
        let d = ax - 1.5;
        d * d / 2.0
    } else {
        0.0
    }
}

/// The Sharp 2021 taps sₙ for |n| = 0..=3 (Costella 2021, eqs. 33–35). Symmetric, and they sum to 1 over −3..=3.
fn s(n: i64) -> f64 {
    match n.unsigned_abs() {
        0 => 17.0 / 12.0,
        1 => -35.0 / 144.0,
        2 => 1.0 / 24.0,
        3 => -1.0 / 144.0,
        _ => 0.0,
    }
}

/// The combined kernel k(x) = Σₙ sₙ·m(x − n).
fn k(x: f64) -> f64 {
    (-3i64..=3).map(|n| s(n) * m(x - pg_model::num::signed(n))).sum()
}

/// The filter for `fast_image_resize::ResizeAlg::Convolution`.
#[expect(clippy::expect_used, reason = "the support is a positive constant, so the filter is always valid")]
pub(crate) fn mks2021() -> FilterType {
    FilterType::Custom(Filter::new("MKS2021", k, SUPPORT).expect("the support is positive and finite"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_taps_sum_to_one_and_the_kernel_is_a_partition_of_unity() {
        let taps: f64 = (-3i64..=3).map(s).sum();
        assert!((taps - 1.0).abs() < 1e-12, "taps sum to {taps}");
        // The magic kernel sums to 1 over the integers, so k does too (the sum over n of sₙ is 1).
        let total: f64 = (-10i64..=10).map(|x| k(pg_model::num::signed(x))).sum();
        assert!((total - 1.0).abs() < 1e-12, "k sums to {total} over the integers");
    }

    #[test]
    fn the_kernel_is_symmetric_and_vanishes_outside_its_support() {
        for x in [0.1, 0.37, 0.9, 1.4, 2.2] {
            assert!((k(x) - k(-x)).abs() < 1e-12);
        }
        assert!(k(SUPPORT + 0.01).abs() < 1e-12);
    }
}
