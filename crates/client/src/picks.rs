//! The picks of a result list: which results are picked to be saved, and the anchor a Shift-click range starts from.
//! Indices are the list's items. A pick outside the list does nothing.

use std::ops::Range;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Picks {
    picked: Vec<bool>,
    /// The result toggled last, where a Shift-click range starts.
    anchor: Option<usize>,
}

impl Picks {
    /// A list of `len` results, none picked and no anchor.
    pub fn new(len: usize) -> Self {
        Self { picked: vec![false; len], anchor: None }
    }

    /// Show a new list of `len` results: none picked, no anchor.
    pub fn reset(&mut self, len: usize) {
        self.picked.clear();
        self.picked.resize(len, false);
        self.anchor = None;
    }

    pub fn len(&self) -> usize {
        self.picked.len()
    }

    pub fn is_empty(&self) -> bool {
        self.picked.is_empty()
    }

    pub fn is_picked(&self, i: usize) -> bool {
        self.picked.get(i).copied().unwrap_or(false)
    }

    /// How many results are picked.
    pub fn count(&self) -> usize {
        self.picked.iter().filter(|&&p| p).count()
    }

    /// How many of the results in `range` are picked.
    pub fn picked_in(&self, range: Range<usize>) -> usize {
        self.picked[self.clamp(range)].iter().filter(|&&p| p).count()
    }

    /// The picked results, in order.
    pub fn indices(&self) -> impl Iterator<Item = usize> + '_ {
        self.picked.iter().enumerate().filter(|&(_, &p)| p).map(|(i, _)| i)
    }

    /// Pick or unpick `i`; it becomes the anchor of the next `pick_to`. Nothing for an index outside the list.
    pub fn toggle(&mut self, i: usize) {
        if let Some(p) = self.picked.get_mut(i) {
            *p = !*p;
            self.anchor = Some(i);
        }
    }

    /// Shift-click: every result from the anchor to `i` takes the anchor's state, picked or not. Without a usable
    /// anchor, toggles `i`.
    pub fn pick_to(&mut self, i: usize) {
        if i >= self.picked.len() {
            return;
        }
        let Some(anchor) = self.anchor.filter(|&a| a < self.picked.len()) else { return self.toggle(i) };
        let value = self.picked[anchor];
        self.picked[anchor.min(i)..=anchor.max(i)].fill(value);
    }

    /// Pick (or unpick) the results in `range`: a group's Select all and Select none.
    pub fn set(&mut self, range: Range<usize>, picked: bool) {
        let range = self.clamp(range);
        self.picked[range].fill(picked);
    }

    /// `first..first + count`, cut to the results that exist.
    pub fn range(&self, first: usize, count: usize) -> Range<usize> {
        self.clamp(first..first.saturating_add(count))
    }

    fn clamp(&self, range: Range<usize>) -> Range<usize> {
        let len = self.picked.len();
        let start = range.start.min(len);
        start..range.end.clamp(start, len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggle_sets_the_anchor_and_ignores_a_missing_index() {
        let mut p = Picks::new(3);
        p.toggle(1);
        assert!(p.is_picked(1));
        assert_eq!(p.count(), 1);
        p.toggle(9);
        assert_eq!(p.count(), 1);
        p.toggle(1);
        assert!(!p.is_picked(1));
    }

    #[test]
    fn pick_to_fills_from_the_anchor_either_way() {
        let mut p = Picks::new(6);
        p.toggle(2);
        p.pick_to(4);
        assert_eq!(p.indices().collect::<Vec<_>>(), vec![2, 3, 4]);
        // The anchor is still 2: a range back to 0 picks 0..=2 (they are already picked, so it stays so).
        p.pick_to(0);
        assert_eq!(p.indices().collect::<Vec<_>>(), vec![0, 1, 2, 3, 4]);
        // An unpicked anchor clears the range it covers.
        p.toggle(3);
        p.pick_to(5);
        assert_eq!(p.indices().collect::<Vec<_>>(), vec![0, 1, 2]);
    }

    #[test]
    fn pick_to_without_an_anchor_toggles_the_item() {
        let mut p = Picks::new(4);
        p.pick_to(2);
        assert_eq!(p.indices().collect::<Vec<_>>(), vec![2]);
    }

    #[test]
    fn set_and_counts_cut_to_the_list() {
        let mut p = Picks::new(5);
        p.set(3..99, true);
        assert_eq!(p.count(), 2);
        assert_eq!(p.picked_in(0..4), 1);
        assert_eq!(p.range(4, 10), 4..5);
        assert_eq!(p.range(7, 2), 5..5);
        p.set(0..2, true);
        assert_eq!(p.picked_in(0..5), 4);
    }

    #[test]
    fn reset_clears_picks_and_anchor() {
        let mut p = Picks::new(2);
        p.toggle(0);
        p.reset(3);
        assert_eq!(p.len(), 3);
        assert_eq!(p.count(), 0);
        p.pick_to(2);
        assert_eq!(p.count(), 1, "no anchor after reset: a plain toggle");
    }
}
