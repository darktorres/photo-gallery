//! The sort box's choices (`pg_client::sort`): the order of the keys is pinned, and each choice sorts as the box says.

use pg_client::sort::{KEYS, choice_index, sort_choice};
use pg_model::query::{Sort, SortKey};

#[test]
fn the_box_keeps_its_order_and_directions() {
    assert_eq!(
        KEYS.map(SortKey::key),
        [
            "name",
            "extension",
            "size",
            "modified",
            "taken",
            "created",
            "rating",
            "megapixels",
            "width",
            "height",
            "aspect_ratio",
            "duration",
            "frames",
            "borders",
        ]
    );
    assert_eq!(sort_choice(0), Some(Sort { key: SortKey::Name, descending: false }));
    assert_eq!(sort_choice(1), Some(Sort { key: SortKey::Extension, descending: false }));
    assert_eq!(sort_choice(3), Some(Sort { key: SortKey::Modified, descending: true }));
    assert_eq!(sort_choice(14), None);
}

#[test]
fn a_sort_reads_back_to_its_box_index() {
    for index in 0..KEYS.len() {
        let sort = sort_choice(index).expect("in range");
        assert_eq!(choice_index(sort.key), index);
    }
}
