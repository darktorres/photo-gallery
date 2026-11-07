//! Laws of the grid's rows (S-18.2): after a list is set and planned, the rows are consecutive and cover every item
//! once, each item is in the row that holds it, and an item's position and file are each other's inverse.

use pg_client::grid::{Changed, Grid};
use pg_model::grid::{Borders, Entry};
use pg_model::ids::FileId;
use pg_model::media::Ext;
use pg_model::orientation::Orientation;
use pg_model::turns::Turns;
use proptest::prelude::*;

fn entry(id: i64, w: u32, h: u32, ext: Ext) -> Entry {
    Entry {
        id: FileId::new(id),
        mtime: 0,
        width: w,
        height: h,
        duration_ms: Entry::NO_DURATION,
        ext,
        rotation: Turns::NONE,
        pending_turns: Turns::NONE,
        rating: None,
        animated: false,
        orientation: Orientation::UPRIGHT,
        cut: [0; 4],
        borders: Borders::Unknown,
    }
}

/// The entries' sizes, from 1x20 (tall) to 20x1 (wide).
fn item_sizes() -> impl Strategy<Value = Vec<(u32, u32)>> {
    prop::collection::vec((1u32..=20, 1u32..=20), 1..60)
}

/// Entries with file ids 1, 2, ... and the given sizes, all JPEGs.
fn list(sizes: Vec<(u32, u32)>) -> Result<Vec<Entry>, TestCaseError> {
    let jpg = Ext::of("jpg").ok_or_else(|| TestCaseError::fail("jpg is an extension the library holds"))?;
    Ok(sizes.into_iter().zip(1i64..).map(|((w, h), id)| entry(id, w, h, jpg)).collect())
}

proptest! {
    #[test]
    fn the_rows_cover_every_item_once_in_order(
        sizes in item_sizes(),
        width in 200.0f64..1200.0,
        row_height in 50.0f64..300.0,
        spacing in 0.0f64..10.0,
    ) {
        let entries = list(sizes)?;
        let n = entries.len();
        let ids: Vec<FileId> = entries.iter().map(|e| e.id).collect();
        let mut grid = Grid::default();
        grid.set_size(width, row_height, spacing);
        grid.swap_entries(entries);
        let plan = grid.plan(Changed::All);
        grid.commit(plan);

        let rows = grid.rows();
        prop_assert!(!rows.is_empty());
        prop_assert_eq!(rows[0].first, 0);
        for pair in rows.windows(2) {
            prop_assert_eq!(pair[1].first, pair[0].first + pair[0].count, "rows are consecutive");
        }
        let covered: usize = rows.iter().map(|r| r.count).sum();
        prop_assert_eq!(covered, n);
        prop_assert!(rows.iter().all(|r| r.count >= 1));

        for item in 0..n {
            let r = grid.row_index(item);
            prop_assert!(rows[r].first <= item && item < rows[r].first + rows[r].count, "item {item} in row {r}");
            let position = i32::try_from(item).unwrap();
            prop_assert_eq!(grid.position_of(grid.id_at(position)), position);
        }
        prop_assert_eq!(grid.position_of(None), -1);
        prop_assert!(ids.iter().all(|&id| grid.position_of(Some(id)) >= 0));
    }
}
