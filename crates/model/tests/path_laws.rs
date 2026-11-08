//! Laws of the path keys (S-18.2), checked over generated names and folders against the spec the code's docs state
//! (`pg_model::rules`): a name's key orders names case-insensitively, then exactly; a folder's key orders folders
//! component by component; the drive is not part of a folder's key. The reference comparator is the spec itself,
//! written out, not the key functions.

use pg_model::name::Name;
use pg_model::rules::{Merged, PathRules};
use proptest::prelude::*;

/// Name characters: ASCII, the Σ and ς the final-sigma rule touches, İ and ẞ (lower case of two letters), a
/// non-BMP letter, and a space. NUL is not a name character on Windows, and the keys rely on that.
fn name() -> impl Strategy<Value = String> {
    prop::collection::vec(
        prop::sample::select(vec!['a', 'B', 'z', '0', ' ', '-', 'Σ', 'σ', 'ς', 'İ', 'ẞ', 'ß', '𐐀', 'é', 'É']),
        1..8,
    )
    .prop_map(|cs| cs.into_iter().collect())
}

/// The spec's order for a name: its characters in lower case (`char::to_lowercase`, one character at a time), then
/// the name itself.
fn reference(name: &str) -> (String, String) {
    (name.chars().flat_map(char::to_lowercase).collect(), name.to_string())
}

fn component() -> impl Strategy<Value = String> {
    "[a-zA-Z0-9Σσς ]{1,6}"
}

/// A Windows path's folder, merged (its anchor dropped), as the typed rules make it.
#[expect(clippy::panic, reason = "a test helper: a path it cannot parse is a bug in the test")]
fn merged_of(path: &str) -> Merged {
    let Some(parsed) = PathRules::Windows.parse(path) else { panic!("not a Windows path: {path}") };
    PathRules::Windows.merge(&parsed)
}

/// The order key of the file `name` in the folder at `folder` (a Windows path), as the catalog stores it.
fn order_key_in(folder: &str, name: &str) -> Vec<u8> {
    let folder = merged_of(folder);
    PathRules::Windows.order_key(&PathRules::Windows.merged_key(&folder), &Name::from(name))
}

proptest! {
    // The name key orders names as the spec does: byte order of the key is the order of (lower case, exact name).
    #[test]
    fn name_keys_order_as_the_spec_does(a in name(), b in name()) {
        prop_assert_eq!(Name::from(a.as_str()).name_key().cmp(&Name::from(b.as_str()).name_key()), reference(&a).cmp(&reference(&b)));
    }

    // Distinct names give distinct keys: the exact name ends the key, so the order is total (names that differ only
    // in case are still two entries, in a fixed order).
    #[test]
    fn distinct_names_have_distinct_keys(a in name(), b in name()) {
        prop_assert_eq!(a == b, Name::from(a.as_str()).name_key() == Name::from(b.as_str()).name_key());
    }

    // A folder is a key prefix of its subfolders: the folder's files and subfolders come first, then the next name.
    #[test]
    fn a_folder_key_is_a_prefix_of_its_subfolders(parts in prop::collection::vec(component(), 1..4), sub in component()) {
        let merged = parts.join("\\");
        let below = format!("{merged}\\{sub}");
        prop_assert!(PathRules::Windows.merged_key(&merged_of(&below)).starts_with(&PathRules::Windows.merged_key(&merged_of(&merged))));
    }

    // A Posix folder's recursive scope holds every file below it, in a subfolder whose raw name starts with byte 0xFF
    // too (a byte no UTF-8 name starts with), and no file of a sibling whose name extends the folder's.
    #[test]
    fn a_posix_recursive_scope_holds_every_file_below(parts in prop::collection::vec(component(), 1..4), n in name(), tail in "[a-z\u{1}\u{2}]{0,3}") {
        let rules = PathRules::Posix;
        let folder: Vec<Name> = parts.iter().map(|p| Name::from(p.as_str())).collect();
        let (lo, hi) = rules.scope_keys(&folder, true);
        let hi = hi.unwrap_or_default();
        let mut below = folder.clone();
        below.push(Name::from_raw(&[&[0xFF, b'a'][..], tail.as_bytes()].concat()));
        let key = rules.order_key(&rules.folder_key(&below), &Name::from(n.as_str()));
        prop_assert!(lo <= key && key < hi);
        let mut sibling = folder.clone();
        if let Some(last) = sibling.last_mut() {
            *last = Name::from(format!("{}{tail}\u{1}", parts.last().map_or("", String::as_str)).as_str());
        }
        let outside = rules.order_key(&rules.folder_key(&sibling), &Name::from(n.as_str()));
        prop_assert!(outside < lo || outside >= hi);
    }

    // The drive is not part of the key: every drive's copy of a folder is one folder, and its files have one order key.
    #[test]
    fn the_drive_is_not_part_of_the_order_key(parts in prop::collection::vec(component(), 1..4), n in name(), d in "[A-Z]") {
        let path = parts.join("\\");
        let here = format!("D:\\{path}");
        let there = format!("{d}:\\{path}");
        prop_assert_eq!(PathRules::Windows.display_merged(&merged_of(&here)), path.as_str());
        prop_assert_eq!(order_key_in(&here, &n), order_key_in(&there, &n));
    }
}
