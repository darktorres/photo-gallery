//! Which lines from a side's edge are solid, measured as far as they are asked for.

use super::params::Params;
use super::stats::median2;
use super::view::View;

/// Per line: twice its median colour, and whether it is solid (see the `borders` module comment). Each line depends
/// only on itself and the line before, so the lines are measured as far as they are asked for.
#[derive(Default)]
pub(super) struct SolidLines {
    pub(super) medians2: Vec<[i32; 3]>,
    pub(super) solid: Vec<bool>,
}

impl SolidLines {
    /// The bytes its buffers hold.
    pub(super) fn capacity_bytes(&self) -> usize {
        self.medians2.capacity() * size_of::<[i32; 3]>() + self.solid.capacity()
    }

    /// Start over, for another side.
    pub(super) fn clear(&mut self) {
        self.medians2.clear();
        self.solid.clear();
    }

    pub(super) fn measure_to(&mut self, v: &View, n: usize, p: &Params) {
        while self.solid.len() < n {
            let line = self.solid.len();
            let m2 = median2(v, line);
            let off =
                v.line(line).filter(|px| px.iter().zip(&m2).any(|(&x, &m)| (2 * x - m).abs() > 2 * p.tol)).count();
            let off_ok = (pg_model::num::count(off) / pg_model::num::count(v.positions)) < p.solid_off;
            // The colour may change from line to line by tol / 3 at most.
            let jump_ok = self
                .medians2
                .last()
                .is_none_or(|prev| m2.iter().zip(prev).all(|(&m, &q)| 3 * (m - q).abs() <= 2 * p.tol));
            self.medians2.push(m2);
            self.solid.push(off_ok && jump_ok);
        }
    }
}
