//! Laws of the path keys, checked over generated names and folders against the spec the code's docs state
//! (`pg_model::rules`): a Windows name's key is its upcase, unit by unit, so names compare and order by it; a folder's
//! key orders folders component by component; the drive is not part of a folder's key. The reference is the spec
//! itself, written out for the generator's characters, not the key functions.

use pg_model::name::Name;
use pg_model::rules::{Merged, PathRules};
use proptest::prelude::*;

/// Name characters: ASCII, the three sigmas, İ, ẞ and ß (no single-unit case pair among them), a non-BMP letter, é and
/// É, and a space. NUL is not a name character on Windows, and the keys rely on that.
fn name() -> impl Strategy<Value = String> {
    prop::collection::vec(
        prop::sample::select(vec!['a', 'B', 'z', '0', ' ', '-', 'Σ', 'σ', 'ς', 'İ', 'ẞ', 'ß', '𐐀', 'é', 'É']),
        1..8,
    )
    .prop_map(|cs| cs.into_iter().collect())
}

/// The spec's Windows key of a name made of the generator's characters: each character's single-unit uppercase when it
/// lowercases back to it (σ is Σ, é is É, ASCII letters are upper case), and every other character itself (the final ς,
/// whose Σ lowercases to σ; İ, ẞ and ß, with no single-unit uppercase; and the supplementary 𐐀, which NTFS does not
/// fold).
fn reference(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            'σ' => 'Σ',
            'é' => 'É',
            c => c.to_ascii_uppercase(),
        })
        .collect()
}

fn component() -> impl Strategy<Value = String> {
    "[a-zA-Z0-9Σσς ]{1,6}"
}

/// A Windows path's folder, merged (its anchor dropped), as the typed rules make it.
fn merged_of(path: &str) -> std::io::Result<Merged> {
    let Some(parsed) = PathRules::Windows.parse(path) else {
        return Err(std::io::Error::other(format!("not a Windows path: {path}")));
    };
    Ok(PathRules::Windows.merge(&parsed))
}

/// The Windows name key of `name`.
fn key(name: &str) -> Vec<u8> {
    PathRules::Windows.name_key(&Name::from(name))
}

proptest! {
    // The name key orders names as the spec does: byte order of the key is the byte order of the spec's upcase.
    #[test]
    fn name_keys_order_as_the_spec_does(a in name(), b in name()) {
        prop_assert_eq!(key(&a).cmp(&key(&b)), reference(&a).as_bytes().cmp(reference(&b).as_bytes()));
    }

    // Two names have one key exactly when the spec makes them one name (case variants, as NTFS compares them); the
    // catalog's order breaks such ties by the row's id. On Posix every name is its own key.
    #[test]
    fn names_share_a_key_exactly_when_they_are_one_name(a in name(), b in name()) {
        prop_assert_eq!(key(&a) == key(&b), reference(&a) == reference(&b));
        prop_assert_eq!(
            PathRules::Posix.name_key(&Name::from(a.as_str())) == PathRules::Posix.name_key(&Name::from(b.as_str())),
            a == b
        );
    }

    // A folder is a key prefix of its subfolders: the folder's files and subfolders come first, then the next name.
    #[test]
    fn a_folder_key_is_a_prefix_of_its_subfolders(parts in prop::collection::vec(component(), 1..4), sub in component()) {
        let merged = parts.join("\\");
        let below = format!("{merged}\\{sub}");
        prop_assert!(PathRules::Windows.merged_key(&merged_of(&below)?).starts_with(&PathRules::Windows.merged_key(&merged_of(&merged)?)));
    }

    // A Posix folder's recursive scope holds every folder below it, a subfolder whose raw name starts with byte 0xFF
    // too (a byte no UTF-8 name starts with), and no sibling whose name extends the folder's.
    #[test]
    fn a_posix_recursive_scope_holds_every_folder_below(parts in prop::collection::vec(component(), 1..4), sub in name(), tail in "[a-z\u{1}\u{2}]{0,3}") {
        let rules = PathRules::Posix;
        let folder: Vec<Name> = parts.iter().map(|p| Name::from(p.as_str())).collect();
        let (lo, hi) = rules.scope_keys(&folder, true);
        let hi = hi.unwrap_or_default();
        let mut below = folder.clone();
        below.push(Name::from_raw(&[&[0xFF, b'a'][..], tail.as_bytes()].concat()));
        below.push(Name::from(sub.as_str()));
        let key = rules.folder_key(&below);
        prop_assert!(lo <= key && key < hi);
        let mut sibling = folder.clone();
        if let Some(last) = sibling.last_mut() {
            *last = Name::from(format!("{}{tail}\u{1}", parts.last().map_or("", String::as_str)).as_str());
        }
        let outside = rules.folder_key(&sibling);
        prop_assert!(outside < lo || outside >= hi);
    }

    // The drive is not part of the key: every drive's copy of a folder is one folder key.
    #[test]
    fn the_drive_is_not_part_of_the_folder_key(parts in prop::collection::vec(component(), 1..4), d in "[A-Z]") {
        let path = parts.join("\\");
        let here = format!("D:\\{path}");
        let there = format!("{d}:\\{path}");
        prop_assert_eq!(PathRules::Windows.display_merged(&merged_of(&here)?), path.as_str());
        prop_assert_eq!(PathRules::Windows.merged_key(&merged_of(&here)?), PathRules::Windows.merged_key(&merged_of(&there)?));
    }
}
