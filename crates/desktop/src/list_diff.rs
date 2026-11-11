//! The changes that turn one list of keyed rows into another, as a list model reports them: removes, inserts and moves,
//! in an order that keeps every index valid when they are applied one after the other (`Notices::show`). Pure, no Qt: a
//! view that is told a row moved keeps its delegate, its selection and what it shows open, where a reset would start it
//! over. The rows whose key stays but whose content changed are the caller's to find once the order is right (a
//! `dataChanged` each).

use std::collections::BTreeSet;

/// One change of the list, in the numbering of the list as it is when the change is made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Op {
    /// Remove `count` rows from `at` on.
    Remove { at: usize, count: usize },
    /// Insert `count` rows at `at`: the new list's rows `at..at + count`.
    Insert { at: usize, count: usize },
    /// Move the row at `from` to `to` (`to < from`: rows only move up, to their place in the new list).
    Move { from: usize, to: usize },
}

/// The changes that make `old` into `new`, applied in order. Each key is in each list once (the inbox's are its entries'
/// identities, `EntryId::entry`, which a listing never names twice). The removes come first, from the end; then the new
/// list is built from its first row on:
/// a row already there stays, one further down moves up into place, one not there is inserted. Consecutive removes and
/// inserts are one change each. Quadratic in the rows, which are at most the inbox's listed few hundred.
pub(crate) fn diff<K: Ord + Copy>(old: &[K], new: &[K]) -> Vec<Op> {
    let mut ops = Vec::new();
    let wanted: BTreeSet<K> = new.iter().copied().collect();
    debug_assert_eq!(wanted.len(), new.len(), "a key named twice");
    let mut current: Vec<K> = Vec::with_capacity(old.len().max(new.len()));
    // The rows to remove, as runs from the end, so the indices before each run still hold.
    let kept: Vec<bool> = old.iter().map(|key| wanted.contains(key)).collect();
    let gone = |i: usize| kept.get(i) == Some(&false);
    let mut end = old.len();
    while let Some(last) = end.checked_sub(1) {
        if !gone(last) {
            end = last;
            continue;
        }
        let mut at = last;
        while at.checked_sub(1).is_some_and(gone) {
            at -= 1;
        }
        ops.push(Op::Remove { at, count: end - at });
        end = at;
    }
    current.extend(old.iter().zip(&kept).filter(|(_, keep)| **keep).map(|(key, _)| *key));
    for (at, &key) in new.iter().enumerate() {
        if current.get(at) == Some(&key) {
            continue;
        }
        match current.iter().skip(at).position(|k| *k == key) {
            Some(offset) => {
                let from = at + offset;
                let moved = current.remove(from);
                current.insert(at, moved);
                ops.push(Op::Move { from, to: at });
            }
            None => {
                current.insert(at, key);
                match ops.last_mut() {
                    Some(Op::Insert { at: start, count }) if *start + *count == at => *count += 1,
                    _ => ops.push(Op::Insert { at, count: 1 }),
                }
            }
        }
    }
    ops
}

/// Apply `ops` to `rows`, taking an inserted row from `new` (by its index there, as `diff` numbers it): what the model
/// does between each change's begin and end. The model's own rows, so a test checks that the changes come out right.
pub(crate) fn apply<T: Clone>(rows: &mut Vec<T>, op: Op, new: &[T]) {
    match op {
        Op::Remove { at, count } => {
            rows.drain(at..at + count);
        }
        Op::Insert { at, count } => {
            rows.splice(at..at, new.iter().skip(at).take(count).cloned());
        }
        Op::Move { from, to } => {
            let row = rows.remove(from);
            rows.insert(to, row);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `old` made into `new` by `diff`'s changes, checked change by change: each index in range when it is made.
    fn applied(old: &str, new: &str) -> (String, Vec<Op>) {
        let old: Vec<char> = old.chars().collect();
        let new: Vec<char> = new.chars().collect();
        let ops = diff(&old, &new);
        let mut rows = old.clone();
        for &op in &ops {
            match op {
                Op::Remove { at, count } => assert!(at + count <= rows.len(), "{op:?} on {rows:?}"),
                Op::Insert { at, count } => assert!(at <= rows.len() && at + count <= new.len(), "{op:?} on {rows:?}"),
                Op::Move { from, to } => assert!(to < from && from < rows.len(), "{op:?} on {rows:?}"),
            }
            apply(&mut rows, op, &new);
        }
        (rows.into_iter().collect(), ops)
    }

    #[test]
    fn the_same_list_is_no_change() {
        assert_eq!(applied("abc", "abc"), ("abc".to_string(), vec![]));
        assert_eq!(applied("", ""), (String::new(), vec![]));
    }

    /// A new notice at the top is one insert: the rows under it keep their delegates (and the selection).
    #[test]
    fn a_new_row_at_the_top_is_one_insert() {
        assert_eq!(applied("abc", "xabc"), ("xabc".to_string(), vec![Op::Insert { at: 0, count: 1 }]));
        assert_eq!(applied("abc", "xyabc"), ("xyabc".to_string(), vec![Op::Insert { at: 0, count: 2 }]));
    }

    #[test]
    fn rows_gone_are_removed_from_the_end_first_in_runs() {
        let (rows, ops) = applied("abcdef", "acf");
        assert_eq!(rows, "acf");
        assert_eq!(ops, vec![Op::Remove { at: 3, count: 2 }, Op::Remove { at: 1, count: 1 }]);
        assert_eq!(applied("abc", ""), (String::new(), vec![Op::Remove { at: 0, count: 3 }]));
    }

    /// A row posted again moves to the top: one move, not a remove and an insert.
    #[test]
    fn a_row_that_comes_up_is_moved() {
        assert_eq!(applied("abc", "cab"), ("cab".to_string(), vec![Op::Move { from: 2, to: 0 }]));
    }

    #[test]
    fn removes_inserts_and_moves_together() {
        assert_eq!(applied("abcde", "xebzd").0, "xebzd");
        assert_eq!(applied("ab", "ba").0, "ba");
        assert_eq!(applied("", "abc"), ("abc".to_string(), vec![Op::Insert { at: 0, count: 3 }]));
    }

    /// Every list of up to five keys made into every other: the result is the new list, and each change in range.
    #[test]
    fn any_list_becomes_any_other() {
        let keys = ['a', 'b', 'c', 'd', 'e'];
        // Every arrangement of every subset, from a small generator: a permutation of 5 by its number, cut to `n`.
        let lists: Vec<String> = (0..120usize)
            .flat_map(|code| {
                let mut pool: Vec<char> = keys.to_vec();
                let mut code = code;
                let mut order = Vec::new();
                for left in (1..=pool.len()).rev() {
                    order.push(pool.remove(code % left));
                    code /= left;
                }
                (0..=order.len()).map(move |n| order[..n].iter().collect::<String>())
            })
            .collect();
        for old in lists.iter().step_by(7) {
            for new in &lists {
                assert_eq!(&applied(old, new).0, new, "{old} → {new}");
            }
        }
    }
}
