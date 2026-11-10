//! Where a border ends: the seam, and whether it is a straight edge.

use super::params::Params;
use super::view::View;

/// [`refine_seam`]'s widest span: `before + after + 1` lines.
const SEAM_SPAN: usize = 16;

/// The first strong line-to-line change in `[guess - before, guess + after]`.
pub(super) fn refine_seam(v: &View, guess: usize, before: usize, after: usize) -> usize {
    assert!(before + after < SEAM_SPAN, "a seam is looked for in at most {SEAM_SPAN} lines");
    let lo = guess.saturating_sub(before).max(1);
    let hi = (guess + after).min(v.depth - 1);
    if hi < lo {
        return guess;
    }
    // step[k]: the mean (over read positions) of the largest channel change into line lo + k.
    let mut steps = [0.0f64; SEAM_SPAN];
    // The assert keeps the span within `SEAM_SPAN`.
    let Some(steps) = steps.get_mut(..=hi - lo) else {
        return guess;
    };
    for (step, line) in steps.iter_mut().zip(lo..=hi) {
        let total: i64 = v
            .line(line - 1)
            .zip(v.line(line))
            .map(|(a, b)| i64::from(a.iter().zip(&b).map(|(&x, &y)| (y - x).abs()).max().unwrap_or(0)))
            .sum();
        *step = pg_model::num::signed(total) / pg_model::num::count(v.positions);
    }
    let max = steps.iter().copied().fold(0.0, f64::max);
    let strong = 3.0f64.max(0.5 * max);
    steps.iter().position(|&s| s >= strong).map_or(guess, |k| lo + k)
}

/// Per-line horizontal detail per column, 3 × |gray step|, summed down the lines: row `i` of `cum` (`cols` wide) is
/// the sum over lines `first..first + i` at each column step.
#[derive(Default)]
pub(super) struct DetailSums {
    first: usize,
    cols: usize,
    cum: Vec<u32>,
}

impl DetailSums {
    /// The bytes its buffer holds.
    pub(super) fn capacity_bytes(&self) -> usize {
        self.cum.capacity() * size_of::<u32>()
    }

    /// Measure `lines` of `v`, replacing what was measured before.
    pub(super) fn measure(&mut self, v: &View, lines: std::ops::Range<usize>) {
        let cols = v.positions.saturating_sub(1);
        self.first = lines.start;
        self.cols = cols;
        self.cum.clear();
        self.cum.reserve((lines.len() + 1) * cols);
        self.cum.resize(cols, 0);
        for line in lines {
            // The next row: the last one plus the line's steps.
            let last = self.cum.len() - cols;
            self.cum.extend_from_within(last..);
            for (next, step) in self.cum.iter_mut().skip(last + cols).zip(v.steps3(line)) {
                *next += step;
            }
        }
    }

    /// The sums over lines `a..b`, one per column step, in order; `None` for lines outside those measured.
    pub(super) fn between(&self, a: usize, b: usize) -> Option<impl Iterator<Item = u32> + '_> {
        let (upper, lower) = (self.row(a)?, self.row(b)?);
        Some(lower.iter().zip(upper).map(|(&b, &a)| b - a))
    }

    /// Row `line - first` of `cum`: the sums over lines `first..line`.
    fn row(&self, line: usize) -> Option<&[u32]> {
        let start = line.checked_sub(self.first)? * self.cols;
        self.cum.get(start..start + self.cols)
    }
}

/// For a candidate first content line `row`: the fraction of columns where it is a sharp edge. A column counts if
/// its colour jumps across `row`, or its detail does (sharp content below a smooth border), comparing the `k` lines
/// above with the `k` below. Real borders end in a straight edge; skies and bokeh don't.
pub(super) fn seam_straightness(v: &View, sums: &DetailSums, row: usize, k: usize, p: &Params) -> f64 {
    let cols = v.positions.saturating_sub(1);
    if cols == 0 {
        return 0.0;
    }
    let (Some(detail_above), Some(detail_below)) =
        (sums.between(row - 1 - k, row - 1), sums.between(row + 1, row + 1 + k))
    else {
        return 0.0;
    };
    // Per column step (`cols` of them; the lines' read pixels are one more).
    // `k` lines: a count past i64::MAX / 6 is past any image.
    let k = i64::try_from(k).unwrap_or(i64::MAX / 6);
    let sharp = v
        .line(row + 1)
        .zip(v.line(row - 2))
        .zip(detail_above.zip(detail_below))
        .filter(|&((below, above), (a, b))| {
            let color = below.iter().zip(&above).any(|(&x, &y)| (x - y).abs() > p.seam_step);
            // Detail below > 2 × detail above + 2, in sums of 3 × gray over k lines.
            let (a, b) = (i64::from(a), i64::from(b));
            color || b > 2 * a + 6 * k
        })
        .count();
    pg_model::num::count(sharp) / pg_model::num::count(cols)
}
