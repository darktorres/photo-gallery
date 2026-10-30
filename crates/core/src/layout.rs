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

/// Greedy justified layout. A row is closed as soon as its items, at `target_height`, fill `width`; it is then
/// scaled to fit exactly, so rows come out at or below the target. The last row keeps the target height.
pub fn justify(aspects: &[f64], width: f64, target_height: f64, spacing: f64) -> Vec<Row> {
    let mut rows = Vec::new();
    let mut first = 0;
    let mut sum = 0.0;
    for (i, &a) in aspects.iter().enumerate() {
        sum += clamp_aspect(a);
        let count = i + 1 - first;
        let gaps = spacing * (count - 1) as f64;
        if sum * target_height + gaps >= width {
            let height = ((width - gaps) / sum).max(1.0);
            rows.push(Row { first, count, height });
            first = i + 1;
            sum = 0.0;
        }
    }
    if first < aspects.len() {
        rows.push(Row { first, count: aspects.len() - first, height: target_height });
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_fill_the_width() {
        let aspects = [1.5, 0.66, 1.0, 1.0, 2.0, 0.75, 1.33, 1.0, 0.5];
        let (width, spacing) = (1000.0, 4.0);
        let rows = justify(&aspects, width, 200.0, spacing);
        assert_eq!(rows.iter().map(|r| r.count).sum::<usize>(), aspects.len());
        for r in &rows[..rows.len() - 1] {
            let used: f64 =
                aspects[r.first..r.first + r.count].iter().map(|&a| clamp_aspect(a) * r.height).sum::<f64>()
                    + spacing * (r.count - 1) as f64;
            assert!((used - width).abs() < 1e-6, "{used}");
            assert!(r.height <= 200.0);
        }
        assert_eq!(rows.last().unwrap().height, 200.0);
    }
}
