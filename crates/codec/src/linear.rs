//! sRGB and linear light (IEC 61966-2-1). The resizer averages in linear light: averaging the encoded values darkens
//! high-contrast detail. Two tables, built once and read by every worker: the
//! 256 decodes of an 8-bit sRGB sample, and the 65,536 encodes of a linear value on a 16-bit grid, each rounded once
//! to 8 bits. Encoding through the 16-bit grid is exact to the rounding of the 8-bit output.

use std::sync::OnceLock;

/// The sRGB electro-optical transfer function: an encoded value in 0–1 to a linear one.
fn eotf(v: f64) -> f64 {
    if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}

/// Its inverse: a linear value in 0–1 to an encoded one (clamped to 0–1 first).
fn oetf(v: f64) -> f64 {
    let v = v.clamp(0.0, 1.0);
    if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
}

/// The 16-bit encode grid's last index.
const GRID_MAX: usize = 65_535;

/// The 8-bit sample of the linear value `k / 65535`: the rounded encoding, the one the encode table holds.
fn sample(k: usize) -> u8 {
    pg_model::num::f64_to_u8((oetf(pg_model::num::count(k) / 65_535.0) * 255.0).round())
}

pub(crate) struct Tables {
    /// `decode[v]` is the linear value of the 8-bit sRGB sample `v`. An array, so a `u8` index is always in range.
    decode: [f32; 256],
    /// `encode[k]` is the 8-bit sRGB sample of the linear value `k / 65535`.
    encode: Vec<u8>,
}

impl Tables {
    /// The linear value (0–1) of an 8-bit sRGB sample.
    #[inline]
    pub(crate) fn decode(&self, v: u8) -> f32 {
        self.decode.get(usize::from(v)).copied().unwrap_or(0.0)
    }

    /// The 8-bit sRGB sample of a linear value, clamped to 0–1 and rounded once.
    #[inline]
    pub(crate) fn encode(&self, v: f32) -> u8 {
        let k = pg_model::num::f64_to_usize(f64::from(v.clamp(0.0, 1.0)) * pg_model::num::count(GRID_MAX) + 0.5);
        self.encode.get(k).copied().unwrap_or(u8::MAX)
    }
}

/// The tables, built on first use. A pass takes the reference once and reads it per pixel.
pub(crate) fn tables() -> &'static Tables {
    static TABLES: OnceLock<Tables> = OnceLock::new();
    TABLES.get_or_init(|| Tables {
        decode: std::array::from_fn(|v| pg_model::num::f64_to_f32(eotf(pg_model::num::count(v) / 255.0))),
        encode: encode_table(),
    })
}

/// The encode table: `encode[k] = sample(k)` for every `k`. The rounded encoding never falls as `k` rises, so each of
/// the 256 samples starts at one `k`, which a binary search finds: 255 searches of about 16 evaluations each, instead of
/// 65,536 evaluations of the power. The table is the same one the direct evaluation gives (a test checks it).
fn encode_table() -> Vec<u8> {
    let mut table = vec![0u8; GRID_MAX + 1];
    // `start` is the least `k` whose sample is at least `c`, from the previous step; from 0 for c = 1 (sample(0) = 0).
    let mut start = 0usize;
    for c in 1..=u8::MAX {
        // The least `k` in `start..` whose sample is at least `c` (`GRID_MAX + 1` if none).
        let (mut lo, mut hi) = (start, GRID_MAX + 1);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if sample(mid) >= c {
                hi = mid;
            } else {
                lo = mid + 1;
            }
        }
        if let Some(run) = table.get_mut(start..lo) {
            run.fill(c - 1);
        }
        start = lo;
    }
    if let Some(rest) = table.get_mut(start..) {
        rest.fill(u8::MAX);
    }
    table
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The table the direct evaluation gives: one `sample` per grid point.
    fn direct_table() -> Vec<u8> {
        (0..=GRID_MAX).map(sample).collect()
    }

    #[test]
    fn the_search_builds_the_table_the_direct_evaluation_does() {
        assert!(encode_table() == direct_table());
    }

    #[test]
    fn every_8_bit_sample_survives_a_round_trip() {
        for v in 0..=u8::MAX {
            assert_eq!(tables().encode(tables().decode(v)), v, "sample {v}");
        }
    }

    #[test]
    fn the_ends_and_the_midpoint_are_where_srgb_puts_them() {
        let t = tables();
        assert_eq!(t.decode(0), 0.0);
        assert!((t.decode(u8::MAX) - 1.0).abs() < 1e-6);
        // Half the light is the sample 188 (linear 0.5 encodes to 0.7354 of 255), not 128.
        assert_eq!(t.encode(0.5), 188);
        assert_eq!(t.encode(-1.0), 0);
        assert_eq!(t.encode(2.0), u8::MAX);
    }
}
