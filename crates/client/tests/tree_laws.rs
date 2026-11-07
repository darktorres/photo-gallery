//! Laws of the folder tree's rows (S-18.2): every folder in a built tree can be revealed, and the row it reveals is the
//! row the path is found at; a folder that is not in the tree is not found.

use pg_client::tree::Tree;
use pg_model::rules::{Merged, PathRules};
use pg_model::tree::TreeBuilder;
use proptest::prelude::*;

/// Folders as paths on one drive: components of one or two letters from a to c, so folders share parents.
fn folders() -> impl Strategy<Value = Vec<String>> {
    prop::collection::vec(prop::collection::vec("[a-c]{1,2}", 1..4), 1..12)
        .prop_map(|paths| paths.into_iter().map(|parts| format!("D:\\{}", parts.join("\\"))).collect())
}

/// The folder a path names, merged (its drive dropped), under Windows rules.
#[expect(clippy::panic, reason = "a test helper: a path it cannot parse is a bug in the test")]
fn merged(path: &str) -> Merged {
    let rules = PathRules::Windows;
    let Some(parsed) = rules.parse(path) else { panic!("not a Windows path: {path}") };
    rules.merge(&parsed)
}

proptest! {
    #[test]
    fn every_folder_reveals_the_row_it_is_found_at(paths in folders()) {
        let rules = PathRules::Windows;
        let mut builder = TreeBuilder::with_capacity(rules, paths.len(), paths.iter().map(String::len).sum(), 12);
        for (i, path) in paths.iter().enumerate() {
            builder.add(i64::try_from(i).unwrap() + 1, &rules.parse(path).unwrap(), 1);
        }
        let mut tree = Tree::default();
        tree.replace(builder.build());
        for path in &paths {
            let folder = merged(path);
            let shown = rules.display_merged(&folder);
            let revealed = tree.reveal(&folder);
            prop_assert!(revealed.is_some(), "{} is not revealed", shown);
            prop_assert_eq!(revealed, tree.row_of_path(&folder), "{}", shown);
        }
        prop_assert_eq!(tree.row_of_path(&merged("D:\\zz\\zz")), None);
    }
}
