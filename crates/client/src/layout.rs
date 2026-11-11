//! Justified grid layout (PLAN §7): rows of images at their aspect ratio, each row scaled to fill the width.
//! Computed in Rust for any row height, so the grid can change size at runtime; the UI only positions delegates.

use std::ops::Range;

/// One row: items `first..first + count`, all `height` px high.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Row {
    pub first: usize,
    pub count: usize,
    pub height: f64,
}

/// The aspect ratio an item is laid out (and shown) at: clamped to 1:20–20:1, so a single extreme panorama or
/// strip can't collapse a row.
pub fn clamp_aspect(aspect: f64) -> f64 {
    if aspect.is_finite() { aspect.clamp(0.05, 20.0) } else { 1.0 }
}

/// Greedy justified layout of items of these aspect ratios, into `rows` (cleared, its capacity kept: relayouts at
/// every resize and size change don't allocate). A row is closed as soon as its items, at `target_height`, fill
/// `width`; it is then scaled to fit exactly, so rows come out at or below the target. The last row keeps the
/// target height. When `rows` must grow, a first pass counts them, so it grows once, to its exact size.
pub fn justify_into(
    rows: &mut Vec<Row>,
    aspects: impl Iterator<Item = f64> + Clone,
    width: f64,
    target_height: f64,
    spacing: f64,
) {
    rows.clear();
    let mut count = 0;
    greedy(aspects.clone(), width, target_height, spacing, |_| count += 1);
    rows.reserve_exact(count);
    greedy(aspects, width, target_height, spacing, |row| rows.push(row));
}

fn greedy(aspects: impl Iterator<Item = f64>, width: f64, target_height: f64, spacing: f64, mut row: impl FnMut(Row)) {
    let mut first = 0;
    let mut sum = 0.0;
    let mut n = 0;
    for (i, a) in aspects.enumerate() {
        n = i + 1;
        sum += clamp_aspect(a);
        let count = i + 1 - first;
        let gaps = spacing * pg_model::num::count(count - 1);
        if sum * target_height + gaps >= width {
            let height = ((width - gaps) / sum).max(1.0);
            row(Row { first, count, height });
            first = i + 1;
            sum = 0.0;
        }
    }
    if first < n {
        row(Row { first, count: n - first, height: target_height });
    }
}

/// Where an item is in a laid-out grid: its left edge and top, and its size, in pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Frame {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// The frames of a justified grid of items of these aspect ratios in `width` (the rows of `justify_into`, stacked from
/// the top left, `spacing` apart both ways), and the grid's height. One frame per item, in order; the items of a row
/// are as wide as their clamped aspect at the row's height. With no width or no height, every frame is empty.
pub fn frames(aspects: &[f64], width: f64, target_height: f64, spacing: f64) -> (Vec<Frame>, f64) {
    if width <= 0.0 || target_height <= 0.0 {
        return (vec![Frame::default(); aspects.len()], 0.0);
    }
    let mut rows = Vec::new();
    justify_into(&mut rows, aspects.iter().copied(), width, target_height, spacing);
    let mut frames = Vec::with_capacity(aspects.len());
    let mut y = 0.0;
    for row in &rows {
        let mut x = 0.0;
        // A row holds items of `aspects`, as `justify_into` made it of them.
        for &aspect in aspects.get(row.first..row.first + row.count).unwrap_or_default() {
            let w = clamp_aspect(aspect) * row.height;
            frames.push(Frame { x, y, width: w, height: row.height });
            x += w + spacing;
        }
        y += row.height + spacing;
    }
    let height = if rows.is_empty() { 0.0 } else { y - spacing };
    (frames, height)
}

/// Where the items of a laid-out grid are: the rows, the spacing, and the clamped aspect each item is laid out at
/// (`aspect(i)` for item `i`, of `len` items). It answers the cursor's questions: an item's row, its span in the row,
/// and the item a step away.
pub struct Placement<'a, A> {
    pub rows: &'a [Row],
    pub spacing: f64,
    pub len: usize,
    pub aspect: A,
}

impl<A: Fn(usize) -> f64> Placement<'_, A> {
    /// The row an item is in.
    pub fn row_index(&self, item: usize) -> usize {
        self.rows.partition_point(|r| r.first + r.count <= item)
    }

    /// Left edge and width of an item in its row; `None` for an item past the rows.
    pub fn span(&self, item: usize) -> Option<(f64, f64)> {
        let row = self.rows.get(self.row_index(item))?;
        let w = |i: usize| (self.aspect)(i) * row.height;
        let x: f64 = (row.first..item).map(|i| w(i) + self.spacing).sum();
        Some((x, w(item)))
    }

    /// The item `dx` along the row, or `dy` rows along: in the other row, the item under the centre of this one (or the
    /// row's last item, when the row is shorter). Clamped to the grid, so an item stays where it is when there is no row
    /// that way; `None` for an empty grid.
    pub fn neighbor(&self, item: usize, dx: i32, dy: i32) -> Option<usize> {
        if self.len == 0 || self.rows.is_empty() {
            return None;
        }
        let last = self.len - 1;
        let item = item.min(last);
        if dy == 0 {
            return Some(step(item, dx, last));
        }
        let row = self.row_index(item);
        let target = step(row, dy, self.rows.len() - 1);
        if target == row {
            return Some(item);
        }
        let Some((x, w)) = self.span(item) else { return Some(item) };
        let center = x + w / 2.0;
        let Some(&r) = self.rows.get(target) else { return Some(item) };
        let found = (r.first..r.first + r.count)
            .find(|&i| self.span(i).is_some_and(|(x, w)| center < x + w + self.spacing))
            .unwrap_or(r.first + r.count - 1);
        Some(found.min(last))
    }
}

/// `at` moved `by` places, kept within `0..=last`.
fn step(at: usize, by: i32, last: usize) -> usize {
    let by_abs = usize::try_from(by.unsigned_abs()).unwrap_or(usize::MAX);
    if by < 0 { at.saturating_sub(by_abs) } else { at.saturating_add(by_abs).min(last) }
}

/// How a new layout changes the rows shown, so that a view keeps the delegates of the rows that stay: the rows
/// before and after those whose items differ are kept, the ones between are changed in place, then rows are
/// inserted or removed after them. Rows are told apart by their items' range: a row is kept when it holds the same
/// range, whatever the items in it are now ([`RowChange::updates`] says which to read again).
#[derive(Debug, PartialEq)]
pub struct RowChange {
    /// Rows changed in place, numbered alike in both layouts.
    pub changed: Range<usize>,
    /// Rows inserted, or removed, at `changed.end`.
    pub inserted: usize,
    pub removed: usize,
}

/// What a row kept or changed in place needs to show the new layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RowUpdate {
    /// It holds other items: its range changed.
    pub span: bool,
    /// The same range, but its items changed.
    pub items: bool,
    pub height: bool,
}

impl RowChange {
    pub fn between(old: &[Row], new: &[Row]) -> RowChange {
        let same = |(a, b): &(&Row, &Row)| (a.first, a.count) == (b.first, b.count);
        let before = old.iter().zip(new).take_while(same).count();
        let after = old.iter().skip(before).rev().zip(new.iter().skip(before).rev()).take_while(same).count();
        let (old_end, new_end) = (old.len() - after, new.len() - after);
        RowChange {
            changed: before..before + (old_end - before).min(new_end - before),
            inserted: new_end.saturating_sub(old_end),
            removed: old_end.saturating_sub(new_end),
        }
    }

    /// What the rows of `new` that aren't inserted need, in runs of rows that need the same (rows that need nothing
    /// left out). `items_changed(range)`: whether the items in a range both layouts have a row for changed.
    pub fn updates(
        &self,
        old: &[Row],
        new: &[Row],
        items_changed: impl Fn(Range<usize>) -> bool,
    ) -> Vec<(Range<usize>, RowUpdate)> {
        let mut runs: Vec<(Range<usize>, RowUpdate)> = Vec::new();
        let inserted = self.changed.end..self.changed.end + self.inserted;
        for (n, row) in new.iter().enumerate() {
            if inserted.contains(&n) {
                continue;
            }
            // The rows after the inserted or removed ones are numbered apart by as many.
            let o = if n < inserted.start { n } else { n + self.removed - self.inserted };
            let Some(was) = old.get(o) else { continue };
            let span = (was.first, was.count) != (row.first, row.count);
            let update = RowUpdate {
                span,
                items: !span && items_changed(row.first..row.first + row.count),
                height: was.height != row.height,
            };
            if update == (RowUpdate { span: false, items: false, height: false }) {
                continue;
            }
            match runs.last_mut() {
                Some((range, last)) if range.end == n && *last == update => range.end += 1,
                _ => runs.push((n..n + 1, update)),
            }
        }
        runs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(spans: &[(usize, usize, f64)]) -> Vec<Row> {
        spans.iter().map(|&(first, count, height)| Row { first, count, height }).collect()
    }

    /// The rows the change leaves, applied as a view applies it to the old ones; and the rows read again.
    fn apply(old: &[Row], new: &[Row], c: &RowChange, items: &[usize]) -> (Vec<Row>, Vec<usize>) {
        let mut out = old.to_vec();
        let at = c.changed.end;
        out.splice(at..at + c.removed, new[at..at + c.inserted].iter().copied());
        let mut read = Vec::new();
        for (range, u) in c.updates(old, new, |span| items.iter().any(|i| span.contains(i))) {
            for i in range {
                if u.span {
                    (out[i].first, out[i].count) = (new[i].first, new[i].count);
                }
                if u.height {
                    out[i].height = new[i].height;
                }
                if u.span || u.items {
                    read.push(i);
                }
            }
        }
        (out, read)
    }

    #[test]
    fn row_changes_turn_the_old_rows_into_the_new() {
        let a = rows(&[(0, 3, 100.0), (3, 2, 90.0), (5, 4, 80.0), (9, 3, 100.0)]);
        // The change from `a` to `new` with `items` changed: (changed, inserted, removed), and the rows read again.
        let check = |new: Vec<Row>, items: &[usize], (changed, inserted, removed), read: &[usize]| {
            let c = RowChange::between(&a, &new);
            assert_eq!(c, RowChange { changed, inserted, removed }, "{new:?}");
            assert_eq!(apply(&a, &new, &c, items), (new.clone(), read.to_vec()), "{new:?}");
        };
        // The same rows.
        check(a.clone(), &[], (4..4, 0, 0), &[]);
        // An item rewritten: its row reads its items again.
        check(a.clone(), &[6], (4..4, 0, 0), &[2]);
        // A turn in the second row: it splits (the second half is a row inserted, made anew), the rows after it are
        // kept.
        check(
            rows(&[(0, 3, 100.0), (3, 1, 95.0), (4, 1, 95.0), (5, 4, 80.0), (9, 3, 100.0)]),
            &[3],
            (1..2, 1, 0),
            &[1],
        );
        // Two rows merge, one after them changes height.
        check(rows(&[(0, 3, 100.0), (3, 6, 70.0), (9, 3, 110.0)]), &[], (1..2, 0, 1), &[1]);
        // A resize: every row differs.
        check(rows(&[(0, 4, 90.0), (4, 4, 90.0), (8, 4, 90.0)]), &[], (0..3, 0, 1), &[0, 1, 2]);
        // Heights alone.
        check(rows(&[(0, 3, 101.0), (3, 2, 90.0), (5, 4, 81.0), (9, 3, 100.0)]), &[], (4..4, 0, 0), &[]);
        // A row between changed ones that keeps its range is read again only if its items changed.
        let between = rows(&[(0, 2, 100.0), (2, 3, 90.0), (5, 4, 80.0), (9, 2, 100.0), (11, 1, 50.0)]);
        check(between.clone(), &[], (0..4, 1, 0), &[0, 1, 3]);
        check(between, &[5], (0..4, 1, 0), &[0, 1, 2, 3]);
        // To no rows (no width).
        check(Vec::new(), &[], (0..0, 0, 4), &[]);
        let c = RowChange::between(&[], &a);
        assert_eq!(c, RowChange { changed: 0..0, inserted: 4, removed: 0 });
        assert_eq!(apply(&[], &a, &c, &[]), (a.clone(), Vec::new()));
    }

    #[test]
    fn rows_fill_the_width() {
        let aspects = [1.5, 0.66, 1.0, 1.0, 2.0, 0.75, 1.33, 1.0, 0.5];
        let (width, spacing) = (1000.0, 4.0);
        let mut rows = vec![Row { first: 9, count: 9, height: 9.0 }; 64];
        justify_into(&mut rows, aspects.iter().copied(), width, 200.0, spacing);
        // Cleared first; its capacity kept.
        assert!(rows.capacity() >= 64 && rows[0].first == 0);
        assert_eq!(rows.iter().map(|r| r.count).sum::<usize>(), aspects.len());
        for r in &rows[..rows.len() - 1] {
            let used: f64 =
                aspects[r.first..r.first + r.count].iter().map(|&a| clamp_aspect(a) * r.height).sum::<f64>()
                    + spacing * pg_model::num::count(r.count - 1);
            assert!((used - width).abs() < 1e-6, "{used}");
            assert!(r.height <= 200.0);
        }
        assert_eq!(rows.last().unwrap().height, 200.0);
        // Grown from nothing, to exactly the rows (`reserve_exact` on an empty `Vec` gives what it asks for).
        let mut fresh = Vec::new();
        justify_into(&mut fresh, aspects.iter().copied(), width, 200.0, spacing);
        assert_eq!((fresh.len(), fresh.capacity()), (rows.len(), rows.len()));
        assert_eq!(fresh, rows);
        justify_into(&mut fresh, std::iter::empty(), width, 200.0, spacing);
        assert!(fresh.is_empty());
    }

    #[test]
    fn frames_stack_the_rows_and_fill_their_width() {
        let aspects = [1.5, 0.66, 1.0, 1.0, 2.0, 0.75, 1.33, 1.0, 0.5];
        let (width, spacing, target) = (1000.0, 4.0, 200.0);
        let (laid, height) = frames(&aspects, width, target, spacing);
        assert_eq!(laid.len(), aspects.len());
        let mut rows = Vec::new();
        justify_into(&mut rows, aspects.iter().copied(), width, target, spacing);
        // Each row starts at the left edge, one row-height and a spacing below the one before; its items are as wide
        // as their aspects, and a full row fills the width.
        let mut top = 0.0;
        for r in &rows {
            let row = &laid[r.first..r.first + r.count];
            assert!(row.iter().all(|f| f.y == top && f.height == r.height));
            assert_eq!(row[0].x, 0.0);
            for (f, pair) in row.iter().zip(row.iter().skip(1)) {
                assert!((f.x + f.width + spacing - pair.x).abs() < 1e-9);
            }
            let end = row.last().map_or(0.0, |f| f.x + f.width);
            if r.count > 0 && r != rows.last().unwrap() {
                assert!((end - width).abs() < 1e-6, "{end}");
            }
            top += r.height + spacing;
        }
        assert_eq!(height, top - spacing);
        assert_eq!(rows.last().unwrap().height, target);
        // A frame's width is its clamped aspect at its row's height.
        assert!((laid[1].width - 0.66 * rows[0].height).abs() < 1e-9);
        // Nothing to lay out, and no width to lay it out in.
        assert_eq!(frames(&[], width, target, spacing), (Vec::new(), 0.0));
        assert_eq!(frames(&aspects, 0.0, target, spacing), (vec![Frame::default(); aspects.len()], 0.0));
        assert_eq!(frames(&aspects, width, 0.0, spacing), (vec![Frame::default(); aspects.len()], 0.0));
    }

    #[test]
    fn placement_finds_the_row_and_the_neighbour_under_the_centre() {
        // Row 0: items 0..3 at height 100 (aspects 1, 1, 2). Row 1: items 3..5 at height 50 (aspects 1, 1).
        let rows = rows(&[(0, 3, 100.0), (3, 2, 50.0)]);
        let aspects = [1.0, 1.0, 2.0, 1.0, 1.0];
        let p = Placement { rows: &rows, spacing: 0.0, len: 5, aspect: |i: usize| aspects[i] };
        assert_eq!((p.row_index(2), p.row_index(3)), (0, 1));
        assert_eq!(p.span(2), Some((200.0, 200.0)));
        assert_eq!(p.span(4), Some((50.0, 50.0)));
        // Along the row, clamped at its ends.
        assert_eq!((p.neighbor(1, 1, 0), p.neighbor(0, -1, 0), p.neighbor(4, 1, 0)), (Some(2), Some(0), Some(4)));
        // Down from item 2 (its centre at 300 px): row 1 has no item that far right, so its last one.
        assert_eq!(p.neighbor(2, 0, 1), Some(4));
        // Up from item 3 (its centre at 25 px): item 0 is the first in row 0 with its right edge past it.
        assert_eq!(p.neighbor(3, 0, -1), Some(0));
        // No row that way: the item stays.
        assert_eq!(p.neighbor(3, 0, 1), Some(3));
        let none = Placement { rows: &[], spacing: 0.0, len: 0, aspect: |_: usize| 1.0 };
        assert_eq!(none.neighbor(0, 1, 0), None);
    }
}
