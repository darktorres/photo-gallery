//! A result list in two groups, with its picks: the Yandex panel's (the other sizes, then the similar images). The
//! picks follow their results: when results are taken off ([`Results::drop_where`]), the ones left keep their picks.

use std::ops::Range;

use crate::picks::Picks;

/// The results, the first group's first, and which are picked.
#[derive(Clone, Debug, PartialEq)]
pub struct Results<T> {
    items: Vec<T>,
    /// How many of `items` are the first group's.
    first: usize,
    picks: Picks,
}

impl<T> Default for Results<T> {
    fn default() -> Self {
        Results { items: Vec::new(), first: 0, picks: Picks::default() }
    }
}

impl<T> Results<T> {
    /// The results of the two groups, none picked.
    pub fn new(first: Vec<T>, second: Vec<T>) -> Results<T> {
        let len = first.len();
        let mut items = first;
        items.extend(second);
        let picks = Picks::new(items.len());
        Results { items, first: len, picks }
    }

    /// Every result, the first group's first.
    pub fn items(&self) -> &[T] {
        &self.items
    }

    pub fn get(&self, i: usize) -> Option<&T> {
        self.items.get(i)
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// The first group's results.
    pub fn first(&self) -> &[T] {
        &self.items[..self.first]
    }

    /// The second group's results.
    pub fn second(&self) -> &[T] {
        &self.items[self.first..]
    }

    /// The picks.
    pub fn picks(&self) -> &Picks {
        &self.picks
    }

    /// The picks, to pick or unpick.
    pub fn picks_mut(&mut self) -> &mut Picks {
        &mut self.picks
    }

    /// The first group's indices.
    pub fn first_range(&self) -> Range<usize> {
        0..self.first
    }

    /// The second group's indices.
    pub fn second_range(&self) -> Range<usize> {
        self.first..self.items.len()
    }

    /// The picked results, in order.
    pub fn picked(&self) -> impl Iterator<Item = &T> + '_ {
        self.picks.indices().filter_map(|i| self.items.get(i))
    }

    /// Take off the results `dead` says: the others keep their order, their groups and their picks. False when none was
    /// taken off (nothing changed).
    pub fn drop_where(&mut self, mut dead: impl FnMut(&T) -> bool) -> bool {
        let items = std::mem::take(&mut self.items);
        let before = items.len();
        let mut kept = Vec::with_capacity(before);
        let mut picked = Vec::with_capacity(before);
        let mut first = 0;
        for (i, item) in items.into_iter().enumerate() {
            if dead(&item) {
                continue;
            }
            if i < self.first {
                first += 1;
            }
            picked.push(self.picks.is_picked(i));
            kept.push(item);
        }
        let changed = kept.len() != before;
        self.items = kept;
        if changed {
            self.first = first;
            self.picks.reset(self.items.len());
            for (i, is_picked) in picked.into_iter().enumerate() {
                if is_picked {
                    self.picks.set(i..i + 1, true);
                }
            }
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The picks follow their results when a dead one is taken off: the picked results stay picked, in order.
    #[test]
    fn the_picks_stay_on_their_results_when_a_dead_one_goes() {
        let mut results = Results::new(vec![1, 2], vec![3, 4]);
        results.picks_mut().set(0..1, true);
        results.picks_mut().set(2..4, true);
        assert!(results.drop_where(|&n| n == 2));
        assert_eq!(results.items(), [1, 3, 4]);
        assert_eq!(results.picked().copied().collect::<Vec<_>>(), [1, 3, 4]);
        assert_eq!(results.first(), [1], "result 2 was in the first group; one of it is left");
        assert_eq!(results.second(), [3, 4]);
        assert!(!results.drop_where(|_| false), "nothing taken off, nothing changed");
    }

    #[test]
    fn new_results_are_unpicked_in_their_groups() {
        let results = Results::new(vec!['a'], vec!['b', 'c']);
        assert_eq!((results.first_range(), results.second_range()), (0..1, 1..3));
        assert_eq!(results.picks().count(), 0);
    }
}
