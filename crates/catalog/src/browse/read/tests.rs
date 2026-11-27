use super::grid::{FILE_JOINS, grid_sql};
use super::tree::FOLDER_COUNTS;
use crate::browse::fixtures::{
    by, folder_of, ids, library, merged_folder, mixed_library, sorted_by_sqlite, spelled, under_both_rules,
};
use crate::fixture;
use crate::fixture::raw;
use crate::queue;
use pg_model::facets::Facets;
use pg_query::condition;
use rusqlite::types::Value;

use super::scope::{SCOPE_FROM, scope};
use crate::core::Catalog;
use crate::error::Result;
use pg_model::analysis::{THUMBHASH_MAX, ThumbHash};
use pg_model::date::Day;
use pg_model::detection;
use pg_model::grid::{Borders, Entry};
use pg_model::ids::{FileId, FolderId};
use pg_model::media::Ext;
use pg_model::media::MediaType;
use pg_model::name::Name;
use pg_model::query::{BorderFacet, DateRange, Query, Range, RatingFilter, Shape, Sort, SortKey, Status};
use pg_model::rules::{Merged, PathRules};
use pg_model::state::FileState;
use pg_model::tree::{FolderTree, TreeBuilder};
use pg_model::turns::Turns;
use pg_model::view::{Listing, SavedView};
use pg_testrng::Rng;
use rusqlite::params_from_iter;

/// A folder given as its Windows path text, merged: the spelling the Windows-only tests use.
fn folder(text: &str) -> Merged {
    merged_folder(PathRules::Windows, text)
}

/// The sort key of the file `name` in the folder at `folder_text` (a Windows path), as the grid orders by it: its merged
/// folder's key (`mfolders.key`), then its name's key (`path_name_key`); the id breaks a tie.
fn order_key_of(folder_text: &str, name: &str) -> (Vec<u8>, Vec<u8>) {
    let rules = PathRules::Windows;
    (rules.merged_key(&folder(folder_text)), rules.name_key(&Name::from(name)))
}

/// No image gives a longer ThumbHash than `THUMBHASH_MAX`, and some give one that long: every aspect, with alpha
/// and without.
#[test]
fn thumbhashes_fit_inline() {
    let mut longest = [0, 0];
    for k in 1..=100 {
        for (w, h) in [(100, k), (k, 100)] {
            for alpha in [false, true] {
                let rgba: Vec<u8> = (0..w * h)
                    .flat_map(|i| {
                        let (x, y) = (i % w, i / w);
                        let channel = |v: usize| u8::try_from(v).unwrap();
                        let a = if alpha { channel((x * 7 + y * 3) % 256) } else { 255 };
                        [channel(x * 255 / w), channel(y * 255 / h), channel((x ^ y) * 9 % 256), a]
                    })
                    .collect();
                let hash = thumbhash::rgba_to_thumb_hash(w, h, &rgba);
                assert!(hash.len() <= THUMBHASH_MAX, "{w} × {h}, alpha {alpha}: {}", hash.len());
                let kept = ThumbHash::new(&hash).unwrap();
                assert_eq!(kept.as_slice(), &hash[..]);
                longest[usize::from(alpha)] = longest[usize::from(alpha)].max(hash.len());
            }
        }
    }
    assert_eq!(longest, [THUMBHASH_MAX - 1, THUMBHASH_MAX]);
    assert!(ThumbHash::new(&[0; THUMBHASH_MAX + 1]).is_none());
}

#[test]
fn the_cut_is_described_as_shown() {
    // Stored 736 × 1593 with bars of 163 at the top and 131 at the bottom, shown turned a quarter clockwise
    // (orientation 6: upright 1593 × 736): the top bar is on the right.
    let mut e = Entry {
        id: FileId::new(1),
        mtime: 0,
        turns_queued: 0,
        width: 1593,
        height: 736,
        duration_ms: Entry::NO_DURATION,
        ext: Ext::of("jpg").unwrap(),
        turns: Turns::new(0),
        pending_turns: Turns::NONE,
        rating: None,
        animated: false,
        orientation: pg_model::orientation::Orientation::new(6).unwrap(),
        cut: [0, 163, 0, 131],
        borders: Borders::Found,
    };
    assert_eq!(e.shown_cut(), Some([131, 0, 163, 0]));
    // A quarter turn more, not yet written: upside down from stored, the top bar at the bottom.
    e.pending_turns = Turns::new(1);
    assert_eq!(e.shown_cut(), Some([0, 131, 0, 163]));
    e.borders = Borders::None;
    assert_eq!(e.shown_cut(), None);
}

// Windows-only: the anchor is a drive letter or the UNC root, which POSIX paths do not have.
#[test]
fn merged_folders_drop_the_anchor() {
    let names = |parts: &[&str]| -> Vec<Name> { parts.iter().map(|p| Name::from(*p)).collect() };
    assert_eq!(folder(r"E:\pinterest\a").parts(), &names(&["pinterest", "a"])[..]);
    assert!(folder("D:").is_empty());
    assert_eq!(folder(r"\\server\share\x").parts(), &names(&["server", "share", "x"])[..]);
}

fn a_file_is_shown_turned_by_what_the_catalog_has_not_yet_put_in_its_pixels(rules: PathRules) {
    let c = library(rules);
    raw::set_placeholder(&c, FileId::new(1), &[1]);
    let state = || c.file_state(FileId::new(1)).unwrap().unwrap();
    let turns = || c.placeholder(FileId::new(1)).unwrap().unwrap().shown_turns;
    let size = u64::try_from(raw::file_size(&c, FileId::new(1))).unwrap();
    assert_eq!(
        state(),
        FileState { mtime: 3, size, turns: Turns::new(0), pending_turns: Turns::new(0), writing: false }
    );
    assert_eq!(c.file_source(FileId::new(1)).unwrap().unwrap().state, state());
    assert_eq!((turns(), c.placeholder(FileId::new(1)).unwrap().unwrap().hash), (Turns::NONE, ThumbHash::new(&[1])));
    // A turn asked for, not yet written.
    c.queue_rotation(FileId::new(1), Turns::new(3)).unwrap();
    assert_eq!((state().pending_turns, turns()), (Turns::new(3), Turns::new(3)));
    // One kept in the catalog since the analysis, and one more asked for.
    raw::set_rotation(&c, FileId::new(1), Turns::new(1));
    c.queue_rotation(FileId::new(1), Turns::new(2)).unwrap();
    assert_eq!((state().turns, state().pending_turns, turns()), (Turns::new(1), Turns::new(1), Turns::new(2)));
    // Being written into the file.
    assert!(!state().writing);
    let job = queue::Job {
        file_id: FileId::new(1),
        write: pg_model::write::Write::Rotation(Turns::new(2)),
        temp: None,
        attempts: 0,
    };
    let temp = rules.parse(&spelled(rules, r"D:\p\b.jpg.tmp")).unwrap();
    c.record_write_temp(&job, &temp, pg_model::write::Write::Rotation(Turns::new(1))).unwrap();
    assert!(state().writing);
    assert_eq!(c.file_state(FileId::new(99)).unwrap(), None);
    assert!(c.placeholder(FileId::new(99)).unwrap().is_none());
}

under_both_rules!(
    a_file_is_shown_turned_by_what_the_catalog_has_not_yet_put_in_its_pixels,
    a_file_is_shown_turned_by_what_the_catalog_has_not_yet_put_in_its_pixels_windows,
    a_file_is_shown_turned_by_what_the_catalog_has_not_yet_put_in_its_pixels_posix
);

fn ratings_and_orders(rules: PathRules) {
    let c = library(rules);
    let all = Query::default();
    let ratings = |r: &[u8]| Query {
        ratings: r.iter().map(|&n| RatingFilter::try_from(n).unwrap()).collect(),
        ..Query::default()
    };
    // Name: both drives' "p" as one folder, by name (Unicode case-insensitive: Ä after b), then "p\sub".
    assert_eq!(ids(&c, "p", true, &all, Sort::default()), [2, 1, 3, 4]);
    assert_eq!(ids(&c, "p", false, &all, Sort::default()), [2, 1, 3]);
    assert_eq!(ids(&c, "p", true, &all, by(SortKey::Name, true)), [4, 3, 1, 2]);
    assert_eq!(ids(&c, "", true, &all, by(SortKey::Size, true)), [4, 1, 3, 2]);
    assert_eq!(ids(&c, "", true, &all, by(SortKey::Modified, true)), [2, 4, 1, 3]);
    assert_eq!(ids(&c, "", true, &ratings(&[0]), by(SortKey::Size, true)), [4, 2]);
    assert_eq!(ids(&c, "", true, &ratings(&[1, 2, 3, 4, 5]), Sort::default()), [1, 3]);
    assert_eq!(ids(&c, "", true, &ratings(&[2]), Sort::default()), [3]);
    assert_eq!(ids(&c, "", true, &ratings(&[0, 2]), Sort::default()), [2, 3, 4]);
    let bmp = c.details(FileId::new(4)).unwrap().unwrap();
    assert!(!bmp.rating_in_file, "BMP keeps its rating in the catalog");
    assert!(c.details(FileId::new(1)).unwrap().unwrap().rating_in_file);
    let found = c.details(FileId::new(2)).unwrap().unwrap();
    assert_eq!(rules.display(&found.path), spelled(rules, r"E:\p\a.png"));
    assert_eq!((found.size, found.rating_in_file), (10, true));
    assert_eq!(found.border_sides, detection::Sides::new());
    assert_eq!(c.details(FileId::new(99)).unwrap(), None);
}

under_both_rules!(ratings_and_orders, ratings_and_orders_windows, ratings_and_orders_posix);

/// The details panel says a rating goes into the file exactly when `set_rating` queues it: not for a video or a file
/// whose analysis failed, and for an image not analyzed yet.
fn the_details_follow_the_write_back_rule(rules: PathRules) {
    let c = mixed_library(rules);
    let in_file = |id: i64| c.details(FileId::new(id)).unwrap().unwrap().rating_in_file;
    assert_eq!([in_file(5), in_file(7), in_file(8)], [false, false, true], "video, analysis failed, not analyzed");
}

under_both_rules!(
    the_details_follow_the_write_back_rule,
    the_details_follow_the_write_back_rule_windows,
    the_details_follow_the_write_back_rule_posix
);

fn folders_in_the_trees_order(rules: PathRules) {
    let c = library(rules);
    // "p-x" sorts before "p\sub" as a whole path ('-' < '\'), but after it in the tree (p's children first).
    crate::fixture::state::add_folder(&c, 4, &spelled(rules, r"D:\p-x")).unwrap();
    fixture::insert(&c, &[fixture::File::new(FileId::new(9), FolderId::new(4), "z.jpg", 1, 1)]).unwrap();
    assert_eq!(ids(&c, "", true, &Query::default(), Sort::default()), [2, 1, 3, 4, 9]);
    let tree = c.folder_tree().unwrap();
    let roots: Vec<String> = tree.children(None).map(|n| tree.name(n).display().into_owned()).collect();
    assert_eq!(roots, ["p", "p-x"]);
}

under_both_rules!(folders_in_the_trees_order, folders_in_the_trees_order_windows, folders_in_the_trees_order_posix);

/// A file directly in a drive root is counted: the tree's total is what "All folders" lists, and its root
/// files are what the empty folder lists without subfolders (the "Drive roots" row).
fn the_tree_counts_what_the_grid_lists(rules: PathRules) {
    let c = library(rules);
    crate::fixture::state::add_folder(&c, 5, &spelled(rules, r"D:\")).unwrap();
    fixture::insert(&c, &[fixture::File::new(FileId::new(9), FolderId::new(5), "root.jpg", 1, 1)]).unwrap();
    let tree = c.folder_tree().unwrap();
    let all = ids(&c, "", true, &Query::default(), Sort::default());
    let roots = ids(&c, "", false, &Query::default(), Sort::default());
    assert_eq!(roots, [9]);
    assert_eq!(tree.total(), i64::try_from(all.len()).unwrap());
    assert_eq!(tree.root_files(), i64::try_from(roots.len()).unwrap());
}

under_both_rules!(
    the_tree_counts_what_the_grid_lists,
    the_tree_counts_what_the_grid_lists_windows,
    the_tree_counts_what_the_grid_lists_posix
);

/// A node as compared: merged path, name, depth, direct and total counts, drives (upper case, by letter), and
/// how many children, in pre-order.
type Flat = (String, String, usize, i64, i64, Vec<String>, usize);

/// The tree as it was built before (kept as the reference): rows (folder id, path, present files) taken in
/// folder id order, nodes found by their lowercased merged path, children sorted by their name's key.
fn reference_tree(rows: &[(i64, String, i64)]) -> Vec<Flat> {
    struct Node {
        name: String,
        merged_path: String,
        depth: usize,
        children: Vec<usize>,
        direct: i64,
        total: i64,
        drives: String,
    }
    let rules = PathRules::Windows;
    let mut rows = rows.to_vec();
    rows.sort_by_key(|r| r.0);
    let (mut nodes, mut roots) = (Vec::<Node>::new(), Vec::<usize>::new());
    // A node is one folder by its key, which lower-cases each character alone (the rules' key, as the writer stores).
    let mut by_key: std::collections::BTreeMap<Vec<u8>, usize> = Default::default();
    for (_, path, count) in &rows {
        let parsed = rules.parse(path).unwrap();
        // The drive a folder is on, as the path spells it ("D:"); a UNC root is no drive.
        let drive = match parsed.anchor.as_bytes() {
            [_, b':'] => parsed.anchor.display().into_owned(),
            _ => String::new(),
        };
        let mut parent: Option<usize> = None;
        let mut merged = String::new();
        for (depth, part) in parsed.parts.iter().enumerate() {
            if !merged.is_empty() {
                merged.push('\\');
            }
            merged.push_str(&part.display());
            let id = *by_key.entry(rules.folder_key(&parsed.parts[..=depth])).or_insert_with(|| {
                nodes.push(Node {
                    name: part.display().into_owned(),
                    merged_path: merged.clone(),
                    depth,
                    children: Vec::new(),
                    direct: 0,
                    total: 0,
                    drives: String::new(),
                });
                let id = nodes.len() - 1;
                match parent {
                    Some(p) => nodes[p].children.push(id),
                    None => roots.push(id),
                }
                id
            });
            let node = &mut nodes[id];
            node.total += count;
            if !drive.is_empty() && !node.drives.split(' ').any(|d| d.eq_ignore_ascii_case(&drive)) {
                if !node.drives.is_empty() {
                    node.drives.push(' ');
                }
                node.drives.push_str(&drive);
            }
            parent = Some(id);
        }
        if let Some(p) = parent {
            nodes[p].direct += count;
        }
    }
    let names: Vec<Vec<u8>> = nodes.iter().map(|n| rules.folder_key(&[Name::from(n.name.as_str())])).collect();
    roots.sort_by(|&a, &b| names[a].cmp(&names[b]));
    for node in &mut nodes {
        node.children.sort_by(|&a, &b| names[a].cmp(&names[b]));
    }
    fn walk(nodes: &[Node], level: &[usize], out: &mut Vec<Flat>) {
        for &n in level {
            let node = &nodes[n];
            let mut drives: Vec<String> =
                node.drives.split(' ').filter(|d| !d.is_empty()).map(str::to_ascii_uppercase).collect();
            drives.sort();
            let (name, path) = (node.name.clone(), node.merged_path.clone());
            out.push((path, name, node.depth, node.direct, node.total, drives, node.children.len()));
            walk(nodes, &node.children, out);
        }
    }
    let mut out = Vec::new();
    walk(&nodes, &roots, &mut out);
    out
}

fn flat(tree: &FolderTree) -> Vec<Flat> {
    (0..tree.len())
        .filter_map(|n| {
            let node = tree.node(n)?;
            let drives = tree.anchors(n).map(|name| name.display().into_owned()).collect();
            let children = tree.children(Some(n)).count();
            let path = tree.rules().display_merged(&tree.merged(n));
            let name = tree.name(n).display().into_owned();
            Some((path, name, pg_model::num::index(node.depth), node.direct, node.total, drives, children))
        })
        .collect()
}

/// Builds a tree of these rows under `rules`, and checks it against its own structure; under Windows rules (whose
/// drives merge) it is checked against the reference too.
fn check_tree(rules: PathRules, rows: &[(i64, String, i64)]) {
    let mut builder = TreeBuilder::with_capacity(rules, rows.len(), 0, 0);
    for (id, path, count) in rows {
        builder.add(*id, &rules.parse(path).unwrap(), *count).unwrap();
    }
    let tree = builder.build();
    if rules == PathRules::Windows {
        assert_eq!(flat(&tree), reference_tree(rows));
    }
    // A drive root's files are in no node, but counted: "All folders" lists every row's files.
    let in_root = |path: &str| rules.parse(path).unwrap().parts.is_empty();
    let root_files: i64 = rows.iter().filter(|(_, path, _)| in_root(path)).map(|r| r.2).sum();
    assert_eq!(tree.root_files(), root_files);
    assert_eq!(tree.total(), rows.iter().map(|r| r.2).sum::<i64>());
    let top: i64 = tree.children(None).map(|n| tree.node(n).unwrap().total).sum();
    assert_eq!(top + root_files, tree.total());
    let mut key = Vec::new();
    for n in 0..tree.len() {
        let merged = tree.merged(n);
        assert_eq!(tree.find(&merged, &mut key), Some(n), "{merged:?}");
        // Upper case finds the folder under Windows rules only: POSIX names are case-sensitive.
        if rules == PathRules::Windows {
            let upper = Merged::from(
                merged
                    .parts()
                    .iter()
                    .map(|p| Name::from(p.display().to_ascii_uppercase().as_str()))
                    .collect::<Vec<_>>(),
            );
            assert_eq!(tree.find(&upper, &mut key), Some(n), "{merged:?}");
        }
        assert_eq!(tree.key(n), rules.merged_key(&merged));
        assert_eq!(tree.multi_anchor(n), tree.anchors(n).count() > 1);
        if let Some(p) = tree.parent(n) {
            assert!(p < n && n < tree.subtree_end(p) && tree.children(Some(p)).any(|c| c == n));
        }
    }
    assert_eq!(tree.find(&rules.merge(&rules.parse("no such folder").unwrap()), &mut key), None);
}

// Windows-only: the random paths are drive letters and UNC roots, and the reference is the Windows tree (the
// drives merged).
#[test]
fn the_tree_is_built_as_before() {
    let named = [r"D:\p\Sub", r"E:\P\sub\x", r"E:\p", r"D:\p-x", r"d:\P", r"\\srv\share\a", r"D:\ΑΣ\b", r"E:\ας"];
    let named = named.into_iter().chain([r"D:\a\b", "D:\\a\\b\u{1}c", "D:", r"D:\İ", "E:\\i\u{307}"]);
    // A deeper folder first: its spelling of `p` wins over `E:\P`'s.
    check_tree(
        PathRules::Windows,
        &named
            .enumerate()
            .map(|(i, p)| (i64::try_from(i).unwrap() + 1, p.to_string(), i64::try_from(i).unwrap() + 1))
            .collect::<Vec<_>>(),
    );
    let mut rng = Rng::new(0x5851_F42D_4C95_7F2D);
    for _ in 0..30 {
        // Distinct paths and ids, as `folders` has them.
        let (mut paths, mut ids) = (std::collections::BTreeSet::new(), std::collections::BTreeSet::new());
        let mut rows: Vec<(i64, String, i64)> = Vec::new();
        for _ in 0..rng.next_u64() % 200 {
            let (path, id, count) = (
                random_file(&mut rng).0,
                i64::try_from(rng.next_u64() % 1000).unwrap(),
                1 + i64::try_from(rng.next_u64() % 5).unwrap(),
            );
            if paths.insert(path.clone()) && ids.insert(id) {
                rows.push((id, path, count));
            }
        }
        check_tree(PathRules::Windows, &rows);
    }
    check_tree(PathRules::Windows, &[]);
}

/// On a real catalog (read-only, under the path rules it records):
/// `PG_TEST_CATALOG=… cargo test -p pg-catalog real_tree -- --ignored`.
#[test]
#[ignore = "needs PG_TEST_CATALOG, a real catalog (opened read-only)"]
fn real_tree_is_built_as_before() {
    let path = std::env::var_os(pg_testkit::inputs::TEST_CATALOG).expect(pg_testkit::inputs::TEST_CATALOG);
    let rules = raw::recorded_rules(std::path::Path::new(&path));
    let c = crate::core::Catalog::open(std::path::Path::new(&path), rules).unwrap();
    let rows: Vec<(i64, String, i64)> =
        raw::rows_of(&c, &FOLDER_COUNTS.replace("(SELECT path FROM", "(SELECT CAST(path AS TEXT) FROM"), [], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        });
    check_tree(rules, &rows);
    let tree = c.folder_tree().unwrap();
    if rules == PathRules::Windows {
        assert_eq!(flat(&tree), reference_tree(&rows));
    }
    eprintln!("{} folders, {} nodes, {} files", rows.len(), tree.len(), tree.total());
}

#[test]
fn name_order() {
    use std::cmp::Ordering::*;
    let cmp = |a: &str, b: &str| {
        PathRules::Windows.name_key(&Name::from(a)).cmp(&PathRules::Windows.name_key(&Name::from(b)))
    };
    // By the upcase, unit by unit (NTFS's comparison, Unicode, unlike SQLite's NOCASE); names in two cases are one key,
    // and the grid breaks the tie by the id.
    assert_eq!(cmp("a.jpg", "B.jpg"), Less);
    assert_eq!(cmp("Ä.jpg", "b.jpg"), Greater);
    assert_eq!(cmp("ä.jpg", "Ä.JPG"), Equal);
    assert_eq!(cmp("B.jpg", "b.jpg"), Equal);
    assert_eq!(cmp("img10", "IMG10"), Equal);
    assert_eq!(cmp("ab", "abc"), Less);
    assert_eq!(cmp("same", "same"), Equal);
    // Upcase order puts `[ \ ] ^ _` and the backtick after the letters, where a lower case put them before.
    assert_eq!(cmp("_a", "b"), Greater);
}

/// The reference order: folders by their parts, then names, each by its upcase, one UTF-16 unit at a time through the
/// table (`pg_model::upcase::upcase`, unit by unit here, where the key encodes it as WTF-8). Empty parts are dropped, as
/// the tree did. Two names with one upcase are equal: the grid orders them by id.
fn reference_order(a: (&str, &str), b: (&str, &str)) -> std::cmp::Ordering {
    let parts_of = |p: &str| -> Vec<Vec<u16>> { folder(p).parts().iter().map(upper).collect() };
    parts_of(a.0).cmp(&parts_of(b.0)).then_with(|| upper(&Name::from(a.1)).cmp(&upper(&Name::from(b.1))))
}

/// A name's upcase, unit by unit, as UTF-16 units: the reference the keys are checked against. UTF-16 unit order and
/// the key's WTF-8 byte order agree except for a supplementary character against U+E000..U+FFFF, which the corpus
/// below has neither of together.
fn upper(part: &Name) -> Vec<u16> {
    part.to_wide().into_iter().map(pg_model::upcase::upcase).collect()
}

/// Paths and names made of the pieces where lowercasing and byte order are tricky.
fn random_file(rng: &mut Rng) -> (String, String) {
    const PIECES: &[&str] = &[
        "a", "A", "b", "B", "z", "ä", "Ä", "ß", "İ", "i", "\u{212A}", "k", "K", "Σ", "σ", "ς", "e\u{301}", "é", "-",
        ".", " ", "1", "10", "\u{1}", "\u{2}", "\u{3}", "ΑΣ", "ας", "日本", "😀",
    ];
    let part =
        |rng: &mut Rng| -> String { (0..1 + rng.next_u64() % 3).map(|_| PIECES[rng.below(PIECES.len())]).collect() };
    let drive = ["D:", "E:", "d:", r"\\srv\share"][rng.below(4)];
    let mut folder = drive.to_string();
    for _ in 0..rng.next_u64() % 4 {
        folder.push('\\');
        folder.push_str(&part(rng));
    }
    (folder, part(rng))
}

// Windows-only: the keys fold case unit by unit and leave the drives out, and the cases named are Windows'.
#[test]
fn order_keys_sort_as_the_reference() {
    let mut rng = Rng::new(0x9E37_79B9_7F4A_7C15);
    let files: Vec<(String, String)> = (0..400).map(|_| random_file(&mut rng)).collect();
    for a in &files {
        for b in &files {
            let key = order_key_of(&a.0, &a.1).cmp(&order_key_of(&b.0, &b.1));
            assert_eq!(key, reference_order((&a.0, &a.1), (&b.0, &b.1)), "{a:?} against {b:?}");
        }
    }
    // The cases named.
    let before = |a: (&str, &str), b: (&str, &str)| assert!(order_key_of(a.0, a.1) < order_key_of(b.0, b.1));
    before((r"D:\p", "z.jpg"), (r"D:\p\sub", "a.jpg"));
    before((r"D:\p\sub", "z.jpg"), (r"D:\p-x", "a.jpg"));
    before((r"E:\P", "a.jpg"), (r"D:\p", "b.jpg"));
    before((r"D:\a\b", "z"), ("D:\\a\\b\u{1}c", "a"));
    // σ upcases to Σ, as NTFS folds it: `ΑΣ` and `ασ` are one folder, on any drive. The final ς keeps its own name
    // (Σ lowercases to σ, and NTFS folds only what round-trips): `ας` is another folder.
    assert_eq!(order_key_of(r"D:\ΑΣ", "a"), order_key_of(r"E:\ασ", "a"));
    assert_ne!(order_key_of(r"D:\ΑΣ", "a"), order_key_of(r"E:\ας", "a"));
}

/// Folder keys are made per UTF-16 unit: each part upcased unit by unit through the table (σ is Σ, ς is ς), escaped,
/// then 0x01. The writer stores these keys and the scope reads them, so the two agree. A name's key is its upcase, as
/// WTF-8. Windows-only: the folder key upcases as Windows does (a POSIX key holds the exact bytes).
#[test]
fn folder_keys_are_made_per_unit() {
    let rules = PathRules::Windows;
    let upcased = |part: &Name| -> Vec<u8> { Name::from_wide(&upper(part)).as_bytes().to_vec() };
    let reference = |parts: &[Name]| -> Vec<u8> {
        let mut key = Vec::new();
        for part in parts {
            for b in upcased(part) {
                match b {
                    0x01 => key.extend([0x02, 0x02]),
                    0x02 => key.extend([0x02, 0x03]),
                    _ => key.push(b),
                }
            }
            key.push(0x01);
        }
        key
    };
    let merged = [
        r"Pinterest\ΑΣ\ΣΑ Σ\aΣb",
        r"ΟΔΥΣΣΕΥΣ\Σ",
        "\u{130}stanbul\\\u{212A}elvin\\ẞ",
        "a\u{1}b\\c\u{2}d\\\u{1}",
        r"\\nas\Photos\2024",
        "",
    ];
    let mut rng = Rng::new(0x2545_F491_4F6C_DD1D);
    let random = (0..400).map(|_| random_file(&mut rng).0);
    for text in merged.into_iter().map(String::from).chain(random) {
        let parts = folder(&text).parts().to_vec();
        assert_eq!(rules.folder_key(&parts), reference(&parts), "{text:?}");
    }
    for name in ["a.JPG", "ΑΣ.jpg", "ας.jpg", "\u{130}.png", ""] {
        assert_eq!(rules.name_key(&Name::from(name)), upcased(&Name::from(name)), "{name:?}");
    }
}

/// Gate G4 of `docs/plans/CATALOG_SIZE_PLAN.md`, on a real catalog's names (read-only, any layout):
/// `PG_TEST_CATALOG=… cargo test -p pg-catalog real_names -- --ignored --nocapture`.
///
/// - **Order:** the names sorted by the new key (each part's upcase, unit by unit) against the old one (each part
///   lower-cased character by character, then the exact name). Every adjacent pair the two put the other way round is
///   listed, and each must be one the new fold explains: the first characters where the two differ are not both ASCII
///   letters or digits (punctuation between the cases, `[ \ ] ^ _` and the backtick; a non-ASCII letter ordered by its
///   capital; a supplementary character), or the two are one name in two cases (the id breaks that tie now, the exact
///   bytes did before).
/// - **Scopes:** every folder against every scope, recursive and not: the reference match (the parts, each upcased) and
///   the key range agree.
#[test]
#[ignore = "needs PG_TEST_CATALOG, a real catalog (opened read-only)"]
fn real_names_sort_and_scope_as_the_reference() {
    let path = std::env::var_os(pg_testkit::inputs::TEST_CATALOG).expect(pg_testkit::inputs::TEST_CATALOG);
    let rules = raw::recorded_rules(std::path::Path::new(&path));
    // The reference order and scopes below are Windows': case folding, drives merged. A POSIX catalog is checked by the
    // rule-generic tests instead.
    assert_eq!(rules, PathRules::Windows, "the reference order and scopes are Windows'");
    let c = crate::core::Catalog::open(std::path::Path::new(&path), rules).unwrap();
    let files: Vec<(String, String)> = raw::rows_of(
        &c,
        "SELECT CAST(fo.path AS TEXT), CAST(f.name AS TEXT) FROM files f JOIN folders fo ON fo.id = f.folder_id",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    );
    // The old key: each part lower-cased character by character, then the name likewise, then its exact bytes.
    let old = |(folder_text, name): &(String, String)| {
        let lower = |s: &str| s.chars().flat_map(char::to_lowercase).collect::<String>();
        let parts: Vec<String> = folder(folder_text).parts().iter().map(|p| lower(&p.display())).collect();
        (parts, lower(name), name.clone())
    };
    let mut by_key = files.clone();
    by_key.sort_by_cached_key(|(folder, name)| order_key_of(folder, name));
    let explained = |a: &str, b: &str| {
        let (la, lb) = (a.to_lowercase(), b.to_lowercase());
        if la == lb {
            return true;
        }
        let first = la.chars().zip(lb.chars()).find(|(x, y)| x != y);
        first.is_none_or(|(x, y)| !(x.is_ascii_alphanumeric() && y.is_ascii_alphanumeric()))
    };
    let mut changed = Vec::new();
    for pair in by_key.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        if old(a) > old(b) {
            let (left, right) = if a.0 == b.0 { (&a.1, &b.1) } else { (&a.0, &b.0) };
            assert!(explained(left, right), "an order change the new fold does not explain: {a:?}, {b:?}");
            changed.push((a.clone(), b.clone()));
        }
    }
    for (a, b) in &changed {
        eprintln!("order changed: {a:?} now before {b:?}");
    }
    // The grid's SQL order of every present file is the Rust sort of the keys (`order_key_of`, then the id).
    let present: Vec<(i64, Vec<u8>, Vec<u8>)> = raw::rows_of(
        &c,
        "SELECT f.id, fo.path, f.name FROM files f JOIN folders fo ON fo.id = f.folder_id WHERE f.missing = 0",
        [],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    );
    let mut by_rust: Vec<(Vec<u8>, Vec<u8>, i64)> = present
        .iter()
        .map(|(id, folder, name)| {
            let parts = rules.parse_bytes(folder).expect("a stored folder parses").parts;
            (rules.folder_key(&parts), rules.name_key(&Name::from_raw(name)), *id)
        })
        .collect();
    by_rust.sort();
    let rust: Vec<i64> = by_rust.into_iter().map(|(_, _, id)| id).collect();
    let sql: Vec<i64> = ids(&c, "", true, &Query::default(), Sort::default());
    assert_eq!(sql, rust, "the SQL order is the Rust sort");
    // Every folder against every scope, recursive and not: the reference match (the parts, each upcased) and the range.
    let folders: Vec<String> = raw::rows_of(&c, "SELECT CAST(path AS TEXT) FROM folders", [], |r| r.get(0));
    let in_range = |scope_text: &str, recursive: bool, folder_text: &str| {
        let (_, values) = scope(c.rules(), &folder_of(&c, scope_text), recursive);
        let key = order_key_of(folder_text, "a").0;
        let blob = |v: &Value| match v {
            Value::Blob(b) => b.clone(),
            _ => unreachable!(),
        };
        key >= blob(&values[0]) && values.get(1).is_none_or(|hi| key < blob(hi))
    };
    let upper_parts = |text: &str| -> Vec<Vec<u16>> { folder(text).parts().iter().map(upper).collect() };
    for scope_folder in &folders {
        let merged = upper_parts(scope_folder);
        for folder_text in &folders {
            let m = upper_parts(folder_text);
            for recursive in [false, true] {
                let inside = m == merged || (recursive && (merged.is_empty() || m.starts_with(&merged)));
                assert_eq!(in_range(scope_folder, recursive, folder_text), inside, "{folder_text} in {scope_folder}");
            }
        }
    }
    eprintln!(
        "{} files, {} folders: {} adjacent pairs changed order, all explained",
        files.len(),
        folders.len(),
        changed.len()
    );
}

// Windows-only: the scopes are ranges of Windows keys, and the cases name drives, a UNC root and case folding.
#[test]
fn scopes_are_key_ranges() {
    let inside = |scope_of: (&str, bool), file: (&str, &str)| {
        let (sql, values) = scope(PathRules::Windows, &folder(scope_of.0), scope_of.1);
        // The scope is a range of merged folder keys: the name doesn't enter it.
        let key = order_key_of(file.0, file.1).0;
        let bound = |v: &Value| match v {
            Value::Blob(b) => b.clone(),
            _ => unreachable!(),
        };
        let lo = bound(&values[0]);
        let hi = values.get(1).map(bound);
        assert_eq!(sql.matches('?').count(), values.len());
        key >= lo && hi.is_none_or(|hi| key < hi)
    };
    // Recursive: the folder and below it, on every drive, in any case; not `p-x`.
    assert!(inside(("p", true), (r"D:\p", "a")));
    assert!(inside(("p", true), (r"E:\P\sub", "a")));
    assert!(!inside(("p", true), (r"D:\p-x", "a")));
    assert!(!inside(("p", true), (r"D:\pa", "a")));
    // Not recursive: the folder's own files only.
    assert!(inside(("p", false), (r"D:\p", "a")));
    assert!(!inside(("p", false), (r"D:\p\sub", "a")));
    // The root: everything, or the drives' own files.
    assert!(inside(("", true), (r"D:\p\sub", "a")));
    assert!(inside(("", false), ("D:", "a")));
    assert!(!inside(("", false), (r"D:\p", "a")));
    // A UNC folder, named as the tree names it.
    assert!(inside((r"srv\share\x", false), (r"\\srv\share\x", "a")));
    // A control byte in a folder name is a different folder.
    assert!(!inside((r"a\b", true), ("D:\\a\\b\u{1}c", "a")));
}

/// Sized by the count: an unfiltered list exactly, a filtered or sorted one without much slack.
fn lists_are_made_at_their_size(rules: PathRules) {
    let c = mixed_library(rules);
    let all = c.list_grid(&folder_of(&c, ""), true, &Query::default(), Sort::default()).unwrap();
    assert_eq!(all.capacity(), all.len());
    let sorted = c.list_grid(&folder_of(&c, ""), true, &Query::default(), by(SortKey::Size, true)).unwrap();
    assert_eq!(sorted.capacity(), sorted.len());
    let one = c
        .list_grid(
            &folder_of(&c, ""),
            true,
            &Query { ratings: vec![RatingFilter::try_from(5).unwrap()], ..Query::default() },
            Sort::default(),
        )
        .unwrap();
    assert!(one.capacity() < all.len() * 3 / 4 || one.capacity() == one.len());
}

under_both_rules!(
    lists_are_made_at_their_size,
    lists_are_made_at_their_size_windows,
    lists_are_made_at_their_size_posix
);

fn lists_read_the_order_index(rules: PathRules) {
    let c = library(rules);
    for (text, recursive) in [("p", true), ("p", false), ("", true)] {
        for sort in [Sort::default(), by(SortKey::Name, true), by(SortKey::Taken, true)] {
            let (scope, values) = scope(c.rules(), &folder_of(&c, text), recursive);
            let unrated = condition(&Query { ratings: vec![RatingFilter::Unrated], ..Query::default() });
            let plan = raw::query_plan(
                &c,
                &grid_sql(&scope, &unrated.sql, sort),
                params_from_iter(values.into_iter().chain(unrated.params)),
            );
            assert!(plan.contains("USING INDEX files_mn"), "{plan}");
            assert!(plan.contains("SEARCH mf USING COVERING INDEX sqlite_autoindex_mfolders_1"), "{plan}");
            assert!(!plan.contains("TEMP B-TREE"), "sorted: {plan}");
        }
        // The list's size, from the indexes alone.
        let (scope, values) = scope(c.rules(), &folder_of(&c, text), recursive);
        let plan = raw::query_plan(
            &c,
            &format!("SELECT count(*) FROM {SCOPE_FROM} WHERE f.missing = 0 AND {scope}"),
            params_from_iter(values),
        );
        assert!(plan.contains("COVERING INDEX files_mn"), "{plan}");
    }
    let plan = raw::query_plan(&c, FOLDER_COUNTS, []);
    assert!(plan.contains("COVERING INDEX files_fm") && !plan.contains("TEMP B-TREE"), "{plan}");
}

under_both_rules!(lists_read_the_order_index, lists_read_the_order_index_windows, lists_read_the_order_index_posix);

/// The Duplicates filter's hash probes use the partial hash indexes (which hold only hashed rows): its `x = y` terms
/// imply `x IS NOT NULL`, so the planner takes them, in both directions of the filter.
fn the_duplicates_filter_reads_the_partial_hash_indexes(rules: PathRules) {
    let c = library(rules);
    for wanted in [true, false] {
        let (scope, values) = scope(c.rules(), &folder_of(&c, ""), true);
        let duplicates = condition(&Query { duplicates: Some(wanted), ..Query::default() });
        let plan = raw::query_plan(
            &c,
            &grid_sql(&scope, &duplicates.sql, Sort::default()),
            params_from_iter(values.into_iter().chain(duplicates.params)),
        );
        assert!(plan.contains("INDEX analysis_file_hash (file_hash=?)"), "{wanted}: {plan}");
        assert!(plan.contains("INDEX analysis_pixel_hash (pixel_hash=?)"), "{wanted}: {plan}");
    }
}

under_both_rules!(
    the_duplicates_filter_reads_the_partial_hash_indexes,
    the_duplicates_filter_reads_the_partial_hash_indexes_windows,
    the_duplicates_filter_reads_the_partial_hash_indexes_posix
);

fn facets_filter(rules: PathRules) {
    let c = mixed_library(rules);
    let q = |f: &dyn Fn(&mut Query)| {
        let mut q = Query::default();
        f(&mut q);
        ids(&c, "", true, &q, Sort::default())
    };
    // `sub`'s two files: `c.bmp` (4) and `Clip.MP4` (5). Windows orders names by their upcase (`C.BMP`, `CLIP.MP4`),
    // POSIX by their bytes (`C` before `c`).
    let sub: &[i64] = if rules == PathRules::Windows { &[4, 5] } else { &[5, 4] };
    let with_sub = |head: &[i64]| -> Vec<i64> { head.iter().chain(sub).copied().collect() };
    assert_eq!(q(&|_| {}), with_sub(&[2, 6, 1, 7, 8, 3]));
    assert_eq!(q(&|q| q.types = vec![MediaType::Video]), [5]);
    assert_eq!(q(&|q| q.types = vec![MediaType::Animated, MediaType::Video]), [6, 5]);
    assert_eq!(q(&|q| q.types = vec![MediaType::Image]), [2, 1, 7, 8, 3, 4]);
    assert_eq!(q(&|q| q.exts = vec!["JPG".into()]), [1, 7, 3]);
    assert_eq!(q(&|q| q.shapes = vec![Shape::Square]), [6]);
    assert_eq!(q(&|q| q.shapes = vec![Shape::Landscape]), with_sub(&[2, 1, 3]));
    assert!(q(&|q| q.shapes = vec![Shape::Portrait]).is_empty());
    assert_eq!(q(&|q| q.megapixels.min = Some(5e-6)), with_sub(&[2, 6, 1, 3]));
    assert_eq!(q(&|q| q.megapixels.max = Some(1e-5)), [6]);
    assert_eq!(q(&|q| q.size_mb = Range { min: Some(20.0 / 1_048_576.0), max: Some(40.0 / 1_048_576.0) }), [1, 3, 4]);
    assert_eq!(q(&|q| q.duration_s.min = Some(10.0)), [5]);
    assert_eq!(q(&|q| q.duplicates = Some(true)), [2, 1, 3, 4]);
    assert_eq!(q(&|q| q.duplicates = Some(false)), [6, 5]);
    assert_eq!(q(&|q| q.status = vec![Status::Failed]), [7]);
    assert_eq!(q(&|q| q.status = vec![Status::Pending]), [8]);
    assert_eq!(q(&|q| q.status = vec![Status::Imported]), with_sub(&[2, 6, 1, 3]));
    let day = |from: &str, to: &str| DateRange { from: Day::parse(from), to: Day::parse(to) };
    assert_eq!(q(&|q| q.taken = day("2024-05-01", "2024-05-01")), [1]);
    assert!(q(&|q| q.taken.from = Day::parse("2024-05-02")).is_empty());
    // A malformed date doesn't filter.
    assert_eq!(q(&|q| q.taken.from = Day::parse("2024-13-45")).len(), 8);
    // The test files' mtimes are FILETIMEs a few hundred ns after 1601.
    assert_eq!(q(&|q| q.modified = day("1600-12-30", "1601-01-02")).len(), 8);
    assert!(q(&|q| q.modified.from = Day::parse("2000-01-01")).is_empty());
    assert_eq!(q(&|q| q.name = "ä".into()), [3]);
    assert_eq!(q(&|q| q.name = "clip".into()), [5]);
    // Facets combine with AND.
    assert_eq!(
        q(&|q| {
            q.exts = vec!["jpg".into()];
            q.duplicates = Some(true)
        }),
        [1, 3]
    );
}

under_both_rules!(facets_filter, facets_filter_windows, facets_filter_posix);

/// "No duplicates" lists nothing while a present, non-pending file owes its pairs (the pairing pass has yet to pair it,
/// so no file is known to have none), and lists the same again once nothing is owed: it waits exactly while the pass has
/// work (`Catalog::has_owed_pairs`).
fn no_duplicates_waits_for_the_pairing_pass(rules: PathRules) {
    let c = mixed_library(rules);
    let no_duplicates =
        |c: &crate::Catalog| ids(c, "", true, &Query { duplicates: Some(false), ..Query::default() }, Sort::default());
    let waits = |c: &crate::Catalog| {
        let waits = no_duplicates(c).is_empty();
        assert_eq!(waits, c.has_owed_pairs().unwrap(), "the list waits while the pass has work");
        waits
    };
    assert_eq!(no_duplicates(&c), [6, 5]);
    assert!(!waits(&c));
    c.conn.execute("INSERT INTO pairs_owed(file_id) VALUES (2)", []).unwrap();
    assert!(waits(&c));
    // Owed by a missing file, or a pending one: the pass can't pair it yet, so it does not hold the list back.
    c.conn.execute("UPDATE files SET missing = 1 WHERE id = 2", []).unwrap();
    assert!(!waits(&c));
    c.conn.execute("UPDATE files SET missing = 0 WHERE id = 2", []).unwrap();
    c.mark_pending(&[pg_model::ids::FileId::new(2)]).unwrap();
    assert!(!waits(&c));
    c.conn.execute("UPDATE files SET pending = 0 WHERE id = 2", []).unwrap();
    assert!(waits(&c));
    c.conn.execute("DELETE FROM pairs_owed WHERE file_id = 2", []).unwrap();
    assert_eq!(no_duplicates(&c), [6, 5]);
}

under_both_rules!(
    no_duplicates_waits_for_the_pairing_pass,
    no_duplicates_waits_for_the_pairing_pass_windows,
    no_duplicates_waits_for_the_pairing_pass_posix
);

/// A picture whose duplicate hashes are owed (`hashes_owed`, stored while duplicates detection was off) is not "not a
/// duplicate": it is not hashed yet, and no picture is known to be unique while any is owed (the pairs' rule: the list
/// waits for the Hashes pass, and is listed once the hashes are made).
fn owed_hashes_are_not_listed_as_not_duplicates(rules: PathRules) {
    let c = mixed_library(rules);
    let no_duplicates =
        |c: &crate::Catalog| ids(c, "", true, &Query { duplicates: Some(false), ..Query::default() }, Sort::default());
    assert_eq!(no_duplicates(&c), [6, 5]);
    c.conn.execute("INSERT INTO hashes_owed(file_id) VALUES (5)", []).unwrap();
    assert!(no_duplicates(&c).is_empty(), "an owed picture is not known to be unique, so none is");
    c.conn.execute("DELETE FROM hashes_owed WHERE file_id = 5", []).unwrap();
    assert_eq!(no_duplicates(&c), [6, 5]);
}

under_both_rules!(
    owed_hashes_are_not_listed_as_not_duplicates,
    owed_hashes_are_not_listed_as_not_duplicates_windows,
    owed_hashes_are_not_listed_as_not_duplicates_posix
);

fn sorts_put_missing_values_last(rules: PathRules) {
    let c = mixed_library(rules);
    let all = Query::default();
    assert_eq!(ids(&c, "", true, &all, by(SortKey::Megapixels, true)), [5, 2, 1, 3, 4, 6, 7, 8]);
    assert_eq!(ids(&c, "", true, &all, by(SortKey::Megapixels, false)), [6, 2, 1, 3, 4, 5, 7, 8]);
    assert_eq!(ids(&c, "", true, &all, by(SortKey::Duration, true)), [5, 2, 6, 1, 7, 8, 3, 4]);
    assert_eq!(ids(&c, "", true, &all, by(SortKey::Taken, true))[0], 1);
}

under_both_rules!(
    sorts_put_missing_values_last,
    sorts_put_missing_values_last_windows,
    sorts_put_missing_values_last_posix
);

fn grid_lists_sort_as_sqlite_did(rules: PathRules) {
    use strum::IntoEnumIterator;
    let c = mixed_library(rules);
    // Ties: four 4×3 images, and no taken date or duration on most.
    for key in SortKey::iter() {
        for descending in [false, true] {
            let sort = by(key, descending);
            assert_eq!(ids(&c, "", true, &Query::default(), sort), sorted_by_sqlite(&c, sort), "{sort:?}");
        }
    }
    // Each entry as read alone.
    for e in c.list_grid(&folder_of(&c, ""), true, &Query::default(), Sort::default()).unwrap() {
        assert_eq!(c.grid_entry(e.id).unwrap(), Some(e));
    }
    let entries = c.list_grid(&folder_of(&c, ""), true, &Query::default(), Sort::default()).unwrap();
    let get = |id: i64| *entries.iter().find(|e| e.id == FileId::new(id)).unwrap();
    let (clip, gif, broken) = (get(5), get(6), get(7));
    assert_eq!((clip.media_type(), clip.duration_ms(), clip.ext.key()), (MediaType::Video, Some(12000), "mp4"));
    assert_eq!((gif.media_type(), gif.duration_ms(), gif.ext.key()), (MediaType::Animated, None, "gif"));
    // Not imported (its analysis failed): no size yet, square.
    assert_eq!((broken.shown_size(), broken.aspect()), ((0, 0), 1.0));
    assert_eq!(c.grid_entry(FileId::new(99)).unwrap(), None);
}

under_both_rules!(
    grid_lists_sort_as_sqlite_did,
    grid_lists_sort_as_sqlite_did_windows,
    grid_lists_sort_as_sqlite_did_posix
);

fn facet_counts(rules: PathRules) {
    let c = mixed_library(rules);
    let videos = Query { types: vec![MediaType::Video], ..Query::default() };
    let f = c.facets(&folder_of(&c, ""), true, &videos).unwrap();
    assert_eq!((f.total, f.matching), (8, 1));
    // A facet's own choices are counted without it; the other facets apply.
    assert_eq!(f.types, [(MediaType::Image, 6), (MediaType::Animated, 1), (MediaType::Video, 1)]);
    assert_eq!(f.exts, [("mp4".to_string(), 1)]);
    let f = c.facets(&folder_of(&c, "p"), false, &Query::default()).unwrap();
    assert_eq!(
        f.exts,
        [("jpg".to_string(), 3), ("gif".to_string(), 1), ("png".to_string(), 1), ("webp".to_string(), 1)]
    );
}

under_both_rules!(facet_counts, facet_counts_windows, facet_counts_posix);

/// The facets as they were before the one pass: a statement per facet, each with its own scan of the folder. The
/// reference the pass is checked against.
fn facets_by_statements(catalog: &Catalog, merged: &Merged, recursive: bool, query: &Query) -> Result<Facets> {
    use pg_query::{Counted, MEDIA_CASE, border_sql, condition_without};
    use strum::IntoEnumIterator;
    let conn = &catalog.conn;
    let (scope, scope_values) = scope(catalog.rules(), merged, recursive);
    let count = |skip: Option<Counted>, select: &str, group: &str| -> Result<Vec<(String, i64)>> {
        let cond = condition_without(query, skip);
        let sql = format!(
            "SELECT {select}, count(*) FROM {SCOPE_FROM} {FILE_JOINS} WHERE f.missing = 0 AND {scope} AND {} {group}",
            cond.sql
        );
        let values = scope_values.iter().cloned().chain(cond.params);
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(values), |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    };
    let total: i64 = conn.query_row(
        &format!("SELECT count(*) FROM {SCOPE_FROM} WHERE f.missing = 0 AND {scope}"),
        rusqlite::params_from_iter(&scope_values),
        |r| r.get(0),
    )?;
    let matching = count(None, "''", "")?.first().map_or(0, |r| r.1);
    let mut types: Vec<(MediaType, i64)> = count(Some(Counted::Types), MEDIA_CASE, "GROUP BY 1")?
        .into_iter()
        .filter_map(|(t, n)| Some((MediaType::from_key(&t)?, n)))
        .collect();
    types.sort();
    let exts = count(Some(Counted::Exts), "pg_ext(f.ext)", "GROUP BY 1 ORDER BY 2 DESC, 1")?;
    let cond = condition_without(query, Some(Counted::Borders));
    let mut borders = Vec::new();
    for b in BorderFacet::iter() {
        let sql = format!(
            "SELECT count(*) FROM {SCOPE_FROM} {FILE_JOINS} WHERE f.missing = 0 AND {scope} AND ({}) AND {}",
            cond.sql,
            border_sql(b)
        );
        let values = scope_values.iter().cloned().chain(cond.params.iter().cloned());
        borders.push((b, conn.query_row(&sql, rusqlite::params_from_iter(values), |r| r.get(0))?));
    }
    Ok(Facets { total, matching, types, exts, borders })
}

fn the_facets_in_one_pass_are_the_counts_of_a_statement_per_facet(rules: PathRules) {
    let c = mixed_library(rules);
    let queries = [
        Query::default(),
        Query { types: vec![MediaType::Video], ..Query::default() },
        Query { exts: vec!["jpg".into()], ..Query::default() },
        Query { ratings: vec![RatingFilter::Unrated], ..Query::default() },
        Query { borders: vec![BorderFacet::Black], ..Query::default() },
        Query {
            types: vec![MediaType::Image],
            borders: vec![BorderFacet::None, BorderFacet::Blur],
            ..Query::default()
        },
        Query { exts: vec!["jpg".into(), "png".into()], name: "a".into(), ..Query::default() },
    ];
    for query in &queries {
        for (text, recursive) in [("", true), ("p", false), ("p", true)] {
            let merged = folder_of(&c, text);
            let one_pass = c.facets(&merged, recursive, query).unwrap();
            let statements = facets_by_statements(&c, &merged, recursive, query).unwrap();
            assert_eq!(one_pass, statements, "{merged:?} recursive {recursive}: {query:?}");
        }
    }
}

under_both_rules!(
    the_facets_in_one_pass_are_the_counts_of_a_statement_per_facet,
    the_facets_in_one_pass_are_the_counts_of_a_statement_per_facet_windows,
    the_facets_in_one_pass_are_the_counts_of_a_statement_per_facet_posix
);

/// A folder whose name ends in Σ lists its files: the writer keys it unit by unit, and the scope reads that same key.
/// `ας` is another folder under
/// both rules: on Windows the final ς keeps its own name, as on NTFS (Σ lowercases to σ, and NTFS folds only what
/// round-trips), and `ασ` is the same folder; on POSIX names are exact.
fn a_folder_ending_in_sigma_lists_its_files(rules: PathRules) {
    let c = library(rules);
    crate::fixture::state::add_folder(&c, 9, &spelled(rules, r"D:\ΑΣ")).unwrap();
    fixture::insert(&c, &[fixture::File::new(FileId::new(9), FolderId::new(9), "z.jpg", 1, 1)]).unwrap();
    assert_eq!(ids(&c, "ΑΣ", false, &Query::default(), Sort::default()), [9]);
    assert_eq!(ids(&c, "ΑΣ", true, &Query::default(), Sort::default()), [9]);
    assert_eq!(ids(&c, "ας", false, &Query::default(), Sort::default()), Vec::<i64>::new());
    let small = ids(&c, "ασ", false, &Query::default(), Sort::default());
    match rules {
        PathRules::Windows => assert_eq!(small, [9]),
        PathRules::Posix => assert_eq!(small, Vec::<i64>::new()),
    }
}

under_both_rules!(
    a_folder_ending_in_sigma_lists_its_files,
    a_folder_ending_in_sigma_lists_its_files_windows,
    a_folder_ending_in_sigma_lists_its_files_posix
);

/// Posix folders are case-sensitive: `/photos/Pics` and `/photos/pics` are two folders, listed apart and both in the
/// tree (the Windows-only keys merged them, and the tree dropped one).
#[test]
fn posix_folders_are_case_sensitive_and_listed_apart() {
    let c = Catalog::prepare(std::path::Path::new(":memory:"), crate::Versions::current(), PathRules::Posix).unwrap();
    crate::fixture::state::add_scan(&c, 1, "/").unwrap();
    crate::fixture::state::add_folder(&c, 1, "/photos/Pics").unwrap();
    crate::fixture::state::add_folder(&c, 2, "/photos/pics").unwrap();
    fixture::insert(
        &c,
        &[
            fixture::File::new(FileId::new(1), FolderId::new(1), "a.jpg", 1, 1),
            fixture::File::new(FileId::new(2), FolderId::new(2), "b.jpg", 1, 1),
        ],
    )
    .unwrap();
    assert_eq!(ids(&c, "photos/Pics", false, &Query::default(), Sort::default()), [1]);
    assert_eq!(ids(&c, "photos/pics", false, &Query::default(), Sort::default()), [2]);
    assert_eq!(ids(&c, "photos", true, &Query::default(), Sort::default()), [1, 2]);
    let tree = c.folder_tree().unwrap();
    let photos = tree.children(None).next().unwrap();
    let below: Vec<String> = tree.children(Some(photos)).map(|n| tree.name(n).display().into_owned()).collect();
    assert_eq!(below, ["Pics", "pics"]);
}

/// Windows folder keys fold case: a saved view spelled `P` is the folder `p`, its own files and the subfolder's. (The
/// Posix twin, `posix_folders_are_case_sensitive_and_listed_apart`, keeps the cases apart.)
#[test]
fn a_saved_scope_matches_its_folder_whatever_the_case() {
    let c = library(PathRules::Windows);
    let view =
        SavedView { name: "upper".into(), listing: Listing { folder: folder_of(&c, "P"), ..Listing::default() } };
    c.save_view(&view).unwrap();
    let saved = c.views().unwrap().remove(0);
    assert_eq!(saved.listing.folder, folder_of(&c, "P"), "kept as it was saved");
    let listed = |folder: &Merged, recursive: bool| -> Vec<i64> {
        c.list_grid(folder, recursive, &Query::default(), Sort::default()).unwrap().iter().map(|e| e.id.get()).collect()
    };
    assert_eq!(listed(&saved.listing.folder, false), [2, 1, 3], "D:\\p and E:\\p, as p");
    assert_eq!(listed(&saved.listing.folder, true), [2, 1, 3, 4], "and D:\\p\\sub");
}

fn a_file_keeps_its_exact_name(rules: PathRules) {
    let c = library(rules);
    // A name with no UTF-8 form: a lone surrogate (WTF-8, as Windows holds it; the same bytes are a non-UTF-8 name
    // on Unix). The path the readers give is that name, exactly.
    let name = Name::from_raw(b"x\xED\xA0\x80.jpg");
    c.conn.execute("UPDATE files SET name = ?1 WHERE id = 1", [name.as_bytes()]).unwrap();
    let path = c.file_path(FileId::new(1)).unwrap().unwrap();
    assert_eq!(path.parts.last(), Some(&name));
    assert_eq!(c.file_source(FileId::new(1)).unwrap().unwrap().path, path);
    assert_eq!(c.details(FileId::new(1)).unwrap().unwrap().path, path);
    assert_eq!(c.file_path(FileId::new(99)).unwrap(), None);
}

under_both_rules!(a_file_keeps_its_exact_name, a_file_keeps_its_exact_name_windows, a_file_keeps_its_exact_name_posix);
