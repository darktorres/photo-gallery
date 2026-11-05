//! The new keys (S-17.2) against the reference corpus recorded from the string keys before the change
//! (`reference_keys.tsv`, S-17.0). Every difference must be intended, so the test states them: names must match
//! exactly; a folder key may differ only where the old key lower-cased a word-final Σ to ς (the old keys used
//! `str::to_lowercase` on a whole component, the new ones lower-case per character, as names always did), and the
//! difference is exactly that ς → σ.

use pg_model::name::Name;
use pg_model::rules::PathRules;

const CORPUS: &str = include_str!("reference_keys.tsv");

/// The TSV's escapes: `\\` is a backslash, `\u{N}` a character by its code point.
fn unescape(field: &str) -> String {
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
                let cp = u32::from_str_radix(&hex, 16).expect("hex code point");
                out.push(char::from_u32(cp).expect("a scalar value in the corpus"));
            }
            other => panic!("an unknown escape {other:?} in {field:?}"),
        }
    }
    out
}

fn unhex(text: &str) -> Vec<u8> {
    (0..text.len() / 2).map(|i| u8::from_str_radix(&text[2 * i..2 * i + 2], 16).unwrap()).collect()
}

struct Row {
    kind: String,
    input: String,
    merged: String,
    folder_key: Vec<u8>,
    name_key: Vec<u8>,
    order_key: Vec<u8>,
}

fn rows() -> Vec<Row> {
    CORPUS
        .lines()
        .skip(1)
        .map(|line| {
            let f: Vec<&str> = line.split('\t').collect();
            Row {
                kind: f[0].to_string(),
                input: unescape(f[1]),
                merged: unescape(f[2]),
                folder_key: unhex(f[3]),
                name_key: unhex(f[4]),
                order_key: unhex(f[5]),
            }
        })
        .collect()
}

/// A merged folder's components, as the old code split them.
fn components(merged: &str) -> Vec<Name> {
    merged.split('\\').filter(|p| !p.is_empty()).map(Name::from).collect()
}

#[test]
fn the_corpus_has_its_rows() {
    let rows = rows();
    assert_eq!(rows.len(), 2033);
    assert_eq!(rows.iter().filter(|r| r.kind == "folder").count(), 1015);
}

#[test]
fn every_name_key_is_the_recorded_one() {
    for row in rows().iter().filter(|r| r.kind == "name") {
        let name = Name::from(row.input.as_str());
        assert_eq!(name.name_key(), row.name_key, "name {:?}", row.input);
        // A file's order key is its folder's key then its name's: the corpus used the folder "photos".
        let folder = PathRules::Windows.folder_key(&[Name::from("photos")]);
        assert_eq!(PathRules::Windows.order_key(&folder, &name), row.order_key, "order {:?}", row.input);
    }
}

#[test]
fn a_folder_key_differs_only_by_a_final_sigma() {
    // Both keys with ς written as σ: the only difference the change may make.
    let sigma_only = |key: &[u8]| replace_all(key, &[0xCF, 0x82], &[0xCF, 0x83]);
    let mut changed = Vec::new();
    for row in rows().iter().filter(|r| r.kind == "folder") {
        let new = PathRules::Windows.folder_key(&components(&row.merged));
        if !row.merged.contains('Σ') {
            assert_eq!(new, row.folder_key, "a folder key with no Σ changed: {:?}", row.merged);
            continue;
        }
        if new != row.folder_key {
            assert_eq!(
                sigma_only(&new),
                sigma_only(&row.folder_key),
                "a folder key that differs in more than a final sigma: {:?}",
                row.merged
            );
            changed.push(row.merged.clone());
        }
    }
    // The count is for the ledger: each changed key is a folder whose old lower-casing made a Σ final (ς).
    println!("folder keys changed by the per-character lower case: {}", changed.len());
}

/// `bytes` with each `from` replaced by `to`, left to right, as the corpus's sigmas need.
fn replace_all(bytes: &[u8], from: &[u8], to: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i..].starts_with(from) {
            out.extend_from_slice(to);
            i += from.len();
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    out
}
