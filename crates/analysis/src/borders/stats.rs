//! Medians and runs: the statistics the line measures and the side detection share.

use super::view::View;

/// Twice the per-channel median of a line's read pixels (numpy's: the mean of the middle two of an even count), from
/// histograms.
pub(super) fn median2(v: &View, line: usize) -> [i32; 3] {
    let mut hists = [[0u32; 256]; 3];
    for px in v.line(line) {
        for (hist, value) in hists.iter_mut().zip(px) {
            // A channel value (0..=255) has its bin.
            if let Some(bin) = usize::try_from(value).ok().and_then(|v| hist.get_mut(v)) {
                *bin += 1;
            }
        }
    }
    hists.map(|hist| hist_median2(&hist, v.positions))
}

/// Twice the median of `n` values counted in `hist` (value = index).
pub(super) fn hist_median2(hist: &[u32], n: usize) -> i32 {
    let nth = |k: usize| {
        let mut seen = 0usize;
        // Each bin's value, counted alongside it.
        hist.iter()
            .zip(0i32..)
            .find(|&(&count, _)| {
                seen += pg_model::num::index(count);
                seen > k
            })
            .map_or(0, |(_, value)| value)
    };
    if n == 0 {
        0
    } else if n % 2 == 1 {
        2 * nth(n / 2)
    } else {
        nth(n / 2 - 1) + nth(n / 2)
    }
}

/// The median of `values` (numpy's: the mean of the middle two of an even count), which it reorders. The middle
/// values are selected rather than sorted (`total_cmp`'s order, as a sort by it gives: values it calls equal are
/// the same bits), so it takes linear time and allocates nothing.
pub(super) fn median_f64(values: &mut [f64]) -> Option<f64> {
    let n = values.len();
    if n == 0 {
        return None;
    }
    let (below, &mut upper, _) = values.select_nth_unstable_by(n / 2, f64::total_cmp);
    // For an even count, the lower middle value is the largest of those before the upper one (there are n / 2).
    let lower = if n % 2 == 1 { None } else { below.iter().copied().max_by(f64::total_cmp) };
    Some(lower.map_or(upper, |lower| (lower + upper) / 2.0))
}

/// Length of the leading run of `true`, bridging gaps of up to `max_gap` if the run then resumes for `resume`.
pub(super) fn run_length(mask: &[bool]) -> usize {
    const MAX_GAP: usize = 2;
    const RESUME: usize = 3;
    let mut i = 0;
    while let Some(&m) = mask.get(i) {
        if m {
            i += 1;
            continue;
        }
        let next = mask.iter().skip(i).take(MAX_GAP + 1).position(|&m| m).map(|gap| i + gap);
        match next {
            Some(j) if mask.get(j..j + RESUME).is_some_and(|run| run.iter().all(|&m| m)) => i = j,
            _ => break,
        }
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The median as a sort by `total_cmp` gives it, for every length up to 9 and random values, with ties, both
    /// zeros, infinities and NaNs among them.
    #[test]
    fn median_is_the_sorted_one() {
        fn sorted_median(values: &[f64]) -> Option<f64> {
            let mut v = values.to_vec();
            v.sort_by(f64::total_cmp);
            let n = v.len();
            (n > 0).then(|| if n % 2 == 1 { v[n / 2] } else { (v[n / 2 - 1] + v[n / 2]) / 2.0 })
        }
        let specials = [0.0, -0.0, 1.0, 1.0, f64::INFINITY, f64::NEG_INFINITY, f64::NAN, -f64::NAN, 0.5, 1e-300];
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for _ in 0..20_000 {
            let n = usize::try_from(next() % 10).unwrap();
            let values: Vec<f64> = (0..n)
                .map(|_| match next() % 3 {
                    0 => specials[usize::try_from(next() % u64::try_from(specials.len()).unwrap()).unwrap()],
                    1 => pg_model::num::size(next() % 7),
                    _ => f64::from_bits(next()),
                })
                .collect();
            let want = sorted_median(&values);
            let got = median_f64(&mut values.clone());
            assert_eq!(got.map(f64::to_bits), want.map(f64::to_bits), "{values:?}");
        }
    }

    #[test]
    fn run_length_bridges_short_gaps() {
        let t = true;
        let f = false;
        assert_eq!(run_length(&[t, t, f, t, t, t, f]), 6);
        assert_eq!(run_length(&[t, t, f, f, f, t, t, t]), 2);
        assert_eq!(run_length(&[t, t, f, t, t]), 2);
        assert_eq!(run_length(&[f, t, t, t]), 4);
    }

    #[test]
    fn medians_match_numpy() {
        let mut hist = [0u32; 256];
        for v in [3, 1, 2, 10] {
            hist[v] += 1;
        }
        // numpy.median([1, 2, 3, 10]) = 2.5
        assert_eq!(hist_median2(&hist, 4), 5);
        hist[7] += 1;
        assert_eq!(hist_median2(&hist, 5), 6);
    }
}
