//! The sort box's choices (`pg_client::sort`): the order of the keys is pinned, and each choice sorts as the box says.

use pg_client::sort::{KEYS, choice_index, sort_choice};
use pg_model::query::{Sort, SortKey};

#[test]
fn the_box_keeps_its_order_and_directions() {
    assert_eq!(KEYS.map(SortKey::key), ["name", "size", "modified", "taken", "megapixels", "duration", "borders"]);
    assert_eq!(sort_choice(0), Some(Sort { key: SortKey::Name, descending: false }));
    assert_eq!(sort_choice(2), Some(Sort { key: SortKey::Modified, descending: true }));
    assert_eq!(sort_choice(7), None);
    assert_eq!(sort_choice(-1), None);
}

#[test]
fn a_sort_reads_back_to_its_box_index() {
    for index in 0..KEYS.len() as i32 {
        let sort = sort_choice(index).expect("in range");
        assert_eq!(choice_index(sort.key), index);
    }
}
