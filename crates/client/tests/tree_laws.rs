//! Laws of the folder tree's rows (S-18.2): every folder in a built tree can be revealed, and the row it reveals is the
//! row the path is found at; a folder that is not in the tree is not found. Files directly in the drive roots make a
//! "Drive roots" row (C-58).

use pg_client::tree::{Tree, Visible};
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
    fn every_folder_reveals_the_row_it_is_found_at(paths in folders(), recursive in any::<bool>()) {
        let rules = PathRules::Windows;
        let mut builder = TreeBuilder::with_capacity(rules, paths.len(), paths.iter().map(String::len).sum(), 12);
        for (i, path) in paths.iter().enumerate() {
            builder.add(i64::try_from(i).unwrap() + 1, &rules.parse(path).unwrap(), 1);
        }
        let mut tree = Tree::new(rules);
        tree.replace(builder.build());
        for path in &paths {
            let folder = merged(path);
            let shown = rules.display_merged(&folder);
            let revealed = tree.reveal(&folder, recursive);
            prop_assert!(revealed.is_some(), "{} is not revealed", shown);
            prop_assert_eq!(revealed, tree.row_of(&folder, recursive), "{}", shown);
        }
        prop_assert_eq!(tree.row_of(&merged("D:\\zz\\zz"), recursive), None);
    }

    /// Files directly in the drive roots: the folders' rows are below a "Drive roots" row (row 1) exactly when there
    /// are some, "All" counts them with every folder's files, and the folder `""` is on "All" with its subfolders and
    /// on "Drive roots" without them (on "All" either way when there is no such row). Neither row toggles.
    #[test]
    fn the_drive_roots_row_is_there_when_the_roots_hold_files(
        paths in folders(),
        roots in 0i64..3,
        recursive in any::<bool>(),
    ) {
        let rules = PathRules::Windows;
        let mut builder = TreeBuilder::with_capacity(rules, paths.len() + 1, paths.iter().map(String::len).sum(), 12);
        for (i, path) in paths.iter().enumerate() {
            builder.add(i64::try_from(i).unwrap() + 1, &rules.parse(path).unwrap(), 1);
        }
        if roots > 0 {
            builder.add(0, &rules.parse("D:").unwrap(), roots);
        }
        let mut tree = Tree::new(rules);
        tree.replace(builder.build());
        let has_row = roots > 0;
        prop_assert_eq!(tree.row(0), Some(Visible::All));
        prop_assert_eq!(tree.row(1) == Some(Visible::DriveRoots), has_row);
        prop_assert_eq!(tree.folders().root_files(), roots);
        prop_assert_eq!(tree.folders().total(), i64::try_from(paths.len()).unwrap() + roots);
        let all = Merged::default();
        prop_assert_eq!(tree.row_of(&all, true), Some(0));
        prop_assert_eq!(tree.row_of(&all, false), Some(usize::from(has_row)));
        // The label rule (L-26): the empty folder is "Drive roots" exactly when its row is the second one.
        prop_assert_eq!(tree.shown_as(&all, false) == Some(Visible::DriveRoots), has_row);
        prop_assert_eq!(tree.shown_as(&all, true), Some(Visible::All));
        prop_assert_eq!(tree.reveal(&all, recursive), tree.row_of(&all, recursive));
        prop_assert_eq!(tree.plan_toggle(0), None);
        if has_row {
            prop_assert_eq!(tree.plan_toggle(1), None);
        }
        let first_folder = 1 + usize::from(has_row);
        for path in &paths {
            let row = tree.reveal(&merged(path), recursive);
            prop_assert!(row.is_some_and(|r| r >= first_folder), "{}", path);
        }
    }
}
