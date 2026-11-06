//! Laws of the path keys (S-18.2), checked over generated names and folders against the spec the code's docs state
//! (`pg_model::path`): a name's key orders names case-insensitively, then exactly; a folder's key orders folders
//! component by component; the drive is not part of a folder's key. The reference comparator is the spec itself,
//! written out, not the key functions.

use pg_model::path::{folder_key, merged_path, name_key, order_key_of};
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

proptest! {
    // The name key orders names as the spec does: byte order of the key is the order of (lower case, exact name).
    #[test]
    fn name_keys_order_as_the_spec_does(a in name(), b in name()) {
        prop_assert_eq!(name_key(&a).cmp(&name_key(&b)), reference(&a).cmp(&reference(&b)));
    }

    // Distinct names give distinct keys: the exact name ends the key, so the order is total (names that differ only
    // in case are still two entries, in a fixed order).
    #[test]
    fn distinct_names_have_distinct_keys(a in name(), b in name()) {
        prop_assert_eq!(a == b, name_key(&a) == name_key(&b));
    }

    // A folder is a key prefix of its subfolders: the folder's files and subfolders come first, then the next name.
    #[test]
    fn a_folder_key_is_a_prefix_of_its_subfolders(parts in prop::collection::vec(component(), 1..4), sub in component()) {
        let merged = parts.join("\\");
        let below = format!("{merged}\\{sub}");
        prop_assert!(folder_key(&below).starts_with(&folder_key(&merged)));
    }

    // The drive is not part of the key: every drive's copy of a folder is one folder, and its files have one order key.
    #[test]
    fn the_drive_is_not_part_of_the_order_key(parts in prop::collection::vec(component(), 1..4), n in name(), d in "[A-Z]") {
        let path = parts.join("\\");
        let here = format!("D:\\{path}");
        let there = format!("{d}:\\{path}");
        prop_assert_eq!(merged_path(&here), path.as_str());
        prop_assert_eq!(order_key_of(&here, &n), order_key_of(&there, &n));
    }
}
