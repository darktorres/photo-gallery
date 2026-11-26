//! The keys against the reference corpus (`reference_keys.tsv`). The corpus was recorded from the earlier keys: its
//! folder and name keys are the per-character lower case of the time. Today's keys are the per-unit upcase through the
//! embedded table (`docs/plans/CATALOG_SIZE_PLAN.md`, D1), so the corpus's keys are now the old fold, kept as the
//! record, and the tests state how the new fold may differ from it.
//!
//! - **Equivalence:** two characters are one name under the new fold exactly when they were under the old one, except
//!   the characters listed in `INTENDED`: those where NTFS's single-unit upcase (round-trip mappings only, Unicode 5.1)
//!   and Rust's per-character lower case disagree (the dotted capital I, the Kelvin and Ångström signs, the sharp s and
//!   its capital, the Ohm sign), and any supplementary character (NTFS does not fold outside the BMP). The final sigma,
//!   the long s and the dotless i are their own names under both.
//! - **Structure:** a folder key is each part's name key, escaped, then 0x01, on both rule sets; on Posix the name key is
//!   the exact bytes.
//! - **Order:** the old and the new order of the corpus's names are compared, and the pairs that changed are counted for
//!   the ledger (gate G4 lists them on the real names).
//!
//! The last 26 rows (the Phase 3 widening of the scan plan) are the hand-checked edge cases: the final and initial
//! sigmas, dotted and dotless i, ẞ and ǅ, a ligature, combining marks against precomposed letters (names are not
//! normalized, so those keys differ), and the folder keys of each.

use std::collections::BTreeSet;

use pg_model::name::Name;
use pg_model::rules::PathRules;
use pg_model::upcase::upcase;

const CORPUS: &str = include_str!("reference_keys.tsv");

/// The characters whose fold the new key changed on purpose (see the module doc).
const INTENDED: &[char] = &['İ', '\u{212A}', 'k', 'K', '\u{212B}', 'å', 'Å', 'ß', 'ẞ', '\u{2126}', 'ω', 'Ω'];

/// A corpus the test cannot read: the test fails with the error.
type Fallible<T> = Result<T, Box<dyn std::error::Error>>;

/// The TSV's escapes: `\\` is a backslash, `\u{N}` a character by its code point. An escape the test does not know
/// fails the test.
fn unescape(field: &str) -> Fallible<String> {
    let mut out = String::new();
    let mut chars = field.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('\\') => out.push('\\'),
            Some('u') => {
                assert_eq!(chars.next(), Some('{'), "a \\u escape: {field:?}");
                let hex: String = chars.by_ref().take_while(|&c| c != '}').collect();
                let cp = u32::from_str_radix(&hex, 16).map_err(|e| format!("hex {hex:?} in {field:?}: {e}"))?;
                out.push(char::from_u32(cp).ok_or_else(|| format!("{cp:#x} is not a scalar value, in {field:?}"))?);
            }
            other => return Err(format!("an unknown escape {other:?} in {field:?}").into()),
        }
    }
    Ok(out)
}

struct Row {
    kind: String,
    input: String,
    merged: String,
}

fn rows() -> Fallible<Vec<Row>> {
    CORPUS
        .lines()
        .skip(1)
        .map(|line| -> Fallible<Row> {
            let f: Vec<&str> = line.split('\t').collect();
            let [kind, input, merged, ..] = f.as_slice() else {
                return Err(format!("a row with fewer than three fields: {line:?}").into());
            };
            Ok(Row { kind: (*kind).to_owned(), input: unescape(input)?, merged: unescape(merged)? })
        })
        .collect()
}

/// A merged folder's components, as the old code split them.
fn components(merged: &str) -> Vec<Name> {
    merged.split('\\').filter(|p| !p.is_empty()).map(Name::from).collect()
}

/// The old fold of a character: its per-character lower case.
fn old_fold(c: char) -> String {
    c.to_lowercase().collect()
}

/// The new fold of a character: each UTF-16 unit through the table.
fn new_fold(c: char) -> Vec<u16> {
    let mut buf = [0u16; 2];
    c.encode_utf16(&mut buf).iter().map(|&u| upcase(u)).collect()
}

#[test]
fn the_corpus_has_its_rows() -> Fallible<()> {
    let rows = rows()?;
    assert_eq!(
        rows.len(),
        2059,
        "the corpus has 2059 rows: a change to the reference corpus changes what this test covers"
    );
    assert_eq!(rows.iter().filter(|r| r.kind == "folder").count(), 1023);
    Ok(())
}

#[test]
fn the_fold_changed_only_for_the_intended_characters() -> Fallible<()> {
    let chars: BTreeSet<char> =
        rows()?.iter().flat_map(|r| r.input.chars().chain(r.merged.chars()).collect::<Vec<_>>()).collect();
    let mut unexpected = BTreeSet::new();
    for &a in &chars {
        for &b in &chars {
            let old = old_fold(a) == old_fold(b);
            let new = new_fold(a) == new_fold(b);
            let intended = |c: char| INTENDED.contains(&c) || u32::from(c) > 0xFFFF;
            if old != new && !(intended(a) && intended(b)) {
                unexpected.insert((a, b));
            }
        }
    }
    assert!(unexpected.is_empty(), "characters whose fold changed without being listed: {unexpected:?}");
    Ok(())
}

#[test]
fn a_folder_key_is_each_parts_name_key_escaped_then_its_terminator() -> Fallible<()> {
    let escaped = |key: &[u8]| -> Vec<u8> {
        key.iter()
            .flat_map(|&b| match b {
                0x01 => vec![0x02, 0x02],
                0x02 => vec![0x02, 0x03],
                b => vec![b],
            })
            .collect()
    };
    for row in rows()?.iter().filter(|r| r.kind == "folder") {
        let parts = components(&row.merged);
        for rules in [PathRules::Windows, PathRules::Posix] {
            let expected: Vec<u8> =
                parts.iter().flat_map(|p| [escaped(&rules.name_key(p)), vec![0x01]].concat()).collect();
            assert_eq!(rules.folder_key(&parts), expected, "{rules:?} {:?}", row.merged);
        }
        let posix: Vec<u8> = parts.iter().flat_map(|p| [escaped(p.as_bytes()), vec![0x01]].concat()).collect();
        assert_eq!(PathRules::Posix.folder_key(&parts), posix, "Posix keys are the exact bytes: {:?}", row.merged);
    }
    Ok(())
}

#[test]
fn the_orders_of_the_corpus_names_are_counted() -> Fallible<()> {
    let names: Vec<Name> = rows()?.iter().filter(|r| r.kind == "name").map(|r| Name::from(r.input.as_str())).collect();
    let old = |n: &Name| (n.display().chars().flat_map(char::to_lowercase).collect::<String>(), n.as_bytes().to_vec());
    let new = |n: &Name| (PathRules::Windows.name_key(n), n.as_bytes().to_vec());
    let mut changed = 0usize;
    for (i, a) in names.iter().enumerate() {
        for b in names.iter().skip(i + 1) {
            if old(a).cmp(&old(b)) != new(a).cmp(&new(b)) {
                changed += 1;
            }
        }
    }
    // The count is for the ledger: the upcase order puts `[ \ ] ^ _` and a backtick after the letters, and orders
    // non-ASCII letters by their capitals.
    eprintln!("name pairs whose order changed with the upcase key: {changed} of {}", names.len() * names.len() / 2);
    Ok(())
}
