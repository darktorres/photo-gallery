//! Laws of the folder tree's rows (S-18.2): every folder in a built tree can be revealed, and the row it reveals is the
//! row the path is found at; a folder that is not in the tree is not found.

use pg_client::tree::Tree;
use pg_model::path::merged_path;
use pg_model::tree::TreeBuilder;
use proptest::prelude::*;

/// Folders as merged paths on one drive: components of one or two letters from a to c, so folders share parents.
fn folders() -> impl Strategy<Value = Vec<String>> {
    prop::collection::vec(prop::collection::vec("[a-c]{1,2}", 1..4), 1..12)
        .prop_map(|paths| paths.into_iter().map(|parts| format!("D:\\{}", parts.join("\\"))).collect())
}

proptest! {
    #[test]
    fn every_folder_reveals_the_row_it_is_found_at(paths in folders()) {
        let mut builder = TreeBuilder::with_capacity(paths.len(), paths.iter().map(String::len).sum(), 12);
        for (i, path) in paths.iter().enumerate() {
            builder.add(i as i64 + 1, path, 1);
        }
        let mut tree = Tree::default();
        tree.replace(builder.build());
        for path in &paths {
            let merged = merged_path(path);
            let revealed = tree.reveal(merged);
            prop_assert!(revealed.is_some(), "{} is not revealed", merged);
            prop_assert_eq!(revealed, tree.row_of_path(merged), "{}", merged);
        }
        prop_assert_eq!(tree.row_of_path("zz\\zz"), None);
    }
}
