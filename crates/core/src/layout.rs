//! Justified grid layout (PLAN §7): rows of images at their aspect ratio, each row scaled to fill the width.
//! Computed in Rust for any row height, so the grid can change size at runtime; the UI only positions delegates.

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
        let gaps = spacing * (count - 1) as f64;
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

#[cfg(test)]
mod tests {
    use super::*;

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
                    + spacing * (r.count - 1) as f64;
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
}
