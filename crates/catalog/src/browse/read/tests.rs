use super::tree::{FOLDER_COUNTS, FOLDER_SIZES, TreeBuilder};
use super::*;
use crate::browse::fixtures::{by, ids, library, mixed_library, sorted_by_sqlite};
use crate::fixture;
use crate::query::{DateRange, MediaType, Orientation, Query, Range, Sort, SortKey, Status};
use crate::writes::queue_rotation;
use pg_model::analysis::{THUMBHASH_MAX, ThumbHash};
use pg_model::detection;
use pg_model::ids::FileId;
use pg_model::media::Ext;
use pg_model::path::folder_key;
use pg_model::path::{components, merged_path, name_key, order_key, order_key_of};
use pg_model::turns::Turns;
use rusqlite::params_from_iter;

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
                        let a = if alpha { ((x * 7 + y * 3) % 256) as u8 } else { 255 };
                        [(x * 255 / w) as u8, (y * 255 / h) as u8, ((x ^ y) * 9 % 256) as u8, a]
                    })
                    .collect();
                let hash = thumbhash::rgba_to_thumb_hash(w, h, &rgba);
                assert!(hash.len() <= THUMBHASH_MAX, "{w} × {h}, alpha {alpha}: {}", hash.len());
                let kept = ThumbHash::new(&hash).unwrap();
                assert_eq!(kept.as_slice(), &hash[..]);
                longest[alpha as usize] = longest[alpha as usize].max(hash.len());
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
        width: 1593,
        height: 736,
        duration_ms: Entry::NO_DURATION,
        ext: Ext::of("jpg").unwrap(),
        rotation: Turns::new(0),
        pending_turns: Turns::NONE,
        rating: 0,
        animated: false,
        orientation: 6,
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

#[test]
fn merged_paths() {
    assert_eq!(merged_path(r"E:\pinterest\a"), r"pinterest\a");
    assert_eq!(merged_path("D:"), "");
    assert_eq!(merged_path(r"\\server\share\x"), r"\\server\share\x");
}

#[test]
fn a_file_is_shown_turned_by_what_the_catalog_has_not_yet_put_in_its_pixels() {
    let c = library();
    c.conn.execute("UPDATE analysis SET thumbhash = x'01' WHERE file_id = 1", []).unwrap();
    let state = || file_state(&c, FileId::new(1)).unwrap().unwrap();
    let turns = || placeholder(&c, FileId::new(1)).unwrap().unwrap().turns;
    let size: i64 = c.conn.query_row("SELECT size FROM files WHERE id = 1", [], |r| r.get(0)).unwrap();
    assert_eq!(state(), FileState { mtime: 3, size, rotation: Turns::new(0), turns: Turns::new(0), writing: false });
    assert_eq!(file_source(&c, FileId::new(1)).unwrap().unwrap().state, state());
    assert_eq!((turns(), placeholder(&c, FileId::new(1)).unwrap().unwrap().hash), (Turns::NONE, ThumbHash::new(&[1])));
    // A turn asked for, not yet written.
    queue_rotation(&c, FileId::new(1), Turns::new(3)).unwrap();
    assert_eq!((state().turns, turns()), (Turns::new(3), Turns::new(3)));
    // One kept in the catalog since the analysis, and one more asked for.
    c.conn.execute("UPDATE files SET rotation = 1 WHERE id = 1", []).unwrap();
    queue_rotation(&c, FileId::new(1), Turns::new(2)).unwrap();
    assert_eq!((state().rotation, state().turns, turns()), (Turns::new(1), Turns::new(1), Turns::new(2)));
    // Being written into the file.
    assert!(!state().writing);
    c.conn.execute(r"UPDATE file_writes SET temp = 'D:\p\b.jpg.tmp', temp_value = 1 WHERE file_id = 1", []).unwrap();
    assert!(state().writing);
    assert_eq!(file_state(&c, FileId::new(99)).unwrap(), None);
    assert!(placeholder(&c, FileId::new(99)).unwrap().is_none());
}

#[test]
fn ratings_and_orders() {
    let c = library();
    let all = Query::default();
    let ratings = |r: &[u8]| Query { ratings: r.to_vec(), ..Query::default() };
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
    let bmp = details(&c, FileId::new(4)).unwrap().unwrap();
    assert!(!bmp.rating_in_file, "BMP keeps its rating in the catalog");
    assert!(details(&c, FileId::new(1)).unwrap().unwrap().rating_in_file);
    assert_eq!(
        details(&c, FileId::new(2)).unwrap(),
        Some(Details {
            path: r"E:\p\a.png".into(),
            size: 10,
            rating_in_file: true,
            border_sides: detection::Sides::new()
        })
    );
    assert_eq!(details(&c, FileId::new(99)).unwrap(), None);
}

#[test]
fn folders_in_the_trees_order() {
    let c = library();
    // "p-x" sorts before "p\sub" as a whole path ('-' < '\'), but after it in the tree (p's children first).
    c.conn.execute("INSERT INTO folders(id, path) VALUES (4, 'D:\\p-x')", []).unwrap();
    fixture::insert(&c, &[fixture::File::new(9, 4, "z.jpg", 1, 1)]);
    assert_eq!(ids(&c, "", true, &Query::default(), Sort::default()), [2, 1, 3, 4, 9]);
    let tree = folder_tree(&c).unwrap();
    let roots: Vec<&str> = tree.children(None).map(|n| tree.name(n)).collect();
    assert_eq!(roots, ["p", "p-x"]);
    // Its strings were made at the size reserved for them, not grown.
    let (_, bytes, components): (usize, usize, usize) =
        c.conn.query_row(FOLDER_SIZES, [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap();
    assert_eq!((tree.paths.capacity(), tree.keys.capacity()), (bytes, bytes + components));
    assert_eq!(tree.nodes.capacity(), tree.nodes.len());
}

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
    let mut rows = rows.to_vec();
    rows.sort_by_key(|r| r.0);
    let (mut nodes, mut roots) = (Vec::<Node>::new(), Vec::<usize>::new());
    let mut by_key: std::collections::BTreeMap<String, usize> = Default::default();
    for (_, path, count) in &rows {
        let drive = if merged_path(path).len() < path.len() { &path[..2] } else { "" };
        let mut parent: Option<usize> = None;
        let mut merged = String::new();
        for (depth, part) in components(merged_path(path)).enumerate() {
            if !merged.is_empty() {
                merged.push('\\');
            }
            merged.push_str(part);
            let id = *by_key.entry(merged.to_lowercase()).or_insert_with(|| {
                nodes.push(Node {
                    name: part.to_string(),
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
            if !drive.is_empty() && !node.drives.split(' ').any(|d| d.eq_ignore_ascii_case(drive)) {
                if !node.drives.is_empty() {
                    node.drives.push(' ');
                }
                node.drives.push_str(drive);
            }
            parent = Some(id);
        }
        if let Some(p) = parent {
            nodes[p].direct += count;
        }
    }
    let names: Vec<Vec<u8>> = nodes.iter().map(|n| folder_key(&n.name)).collect();
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
        .map(|n| {
            let node = tree.node(n);
            let drives = tree.drives(n).split(' ').filter(|d| !d.is_empty()).map(String::from).collect();
            let children = tree.children(Some(n)).count();
            let (path, name) = (tree.merged_path(n).to_string(), tree.name(n).to_string());
            (path, name, node.depth as usize, node.direct, node.total, drives, children)
        })
        .collect()
}

/// Builds a tree of these rows, and checks it against the reference and its own structure.
fn check_tree(rows: &[(i64, String, i64)]) {
    let mut builder = TreeBuilder::default();
    for (id, path, count) in rows {
        builder.add(*id, path, *count);
    }
    let tree = builder.build();
    assert_eq!(flat(&tree), reference_tree(rows));
    // The nodes were counted exactly before they were made.
    assert_eq!(tree.nodes.capacity(), tree.nodes.len());
    let mut key = Vec::new();
    for n in 0..tree.len() {
        assert_eq!(tree.find(tree.merged_path(n), &mut key), Some(n));
        assert_eq!(tree.find(&tree.merged_path(n).to_ascii_uppercase(), &mut key), Some(n), "{}", tree.merged_path(n));
        assert_eq!(tree.key(n), folder_key(tree.merged_path(n)));
        assert_eq!(tree.merged(n), tree.drives(n).contains(' '));
        if let Some(p) = tree.parent(n) {
            assert!(p < n && n < tree.subtree_end(p) && tree.children(Some(p)).any(|c| c == n));
        }
    }
    assert_eq!(tree.find("no such folder", &mut key), None);
}

#[test]
fn the_tree_is_built_as_before() {
    let named = [r"D:\p\Sub", r"E:\P\sub\x", r"E:\p", r"D:\p-x", r"d:\P", r"\\srv\share\a", r"D:\ΑΣ\b", r"E:\ας"];
    let named = named.into_iter().chain([r"D:\a\b", "D:\\a\\b\u{1}c", "D:", r"D:\İ", r"E:\i\u{307}"]);
    // A deeper folder first: its spelling of `p` wins over `E:\P`'s.
    check_tree(&named.enumerate().map(|(i, p)| (i as i64 + 1, p.to_string(), i as i64 + 1)).collect::<Vec<_>>());
    let mut rng = Rng(0x5851_F42D_4C95_7F2D);
    for _ in 0..30 {
        // Distinct paths and ids, as `folders` has them.
        let (mut paths, mut ids) = (std::collections::BTreeSet::new(), std::collections::BTreeSet::new());
        let mut rows: Vec<(i64, String, i64)> = Vec::new();
        for _ in 0..rng.next() % 200 {
            let (path, id, count) = (random_file(&mut rng).0, (rng.next() % 1000) as i64, 1 + (rng.next() % 5) as i64);
            if paths.insert(path.clone()) && ids.insert(id) {
                rows.push((id, path, count));
            }
        }
        check_tree(&rows);
    }
    check_tree(&[]);
}

/// On a real catalog (read-only): `PG_TEST_CATALOG=… cargo test -p pg-catalog real_tree -- --ignored`.
#[test]
#[ignore = "needs PG_TEST_CATALOG, a real catalog (opened read-only)"]
fn real_tree_is_built_as_before() {
    let path = std::env::var_os("PG_TEST_CATALOG").expect("PG_TEST_CATALOG");
    let c = crate::Catalog::open_current(std::path::Path::new(&path)).unwrap();
    let conn = &c.conn;
    let rows: Vec<(i64, String, i64)> = conn
        .prepare(FOLDER_COUNTS)
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    check_tree(&rows);
    let tree = folder_tree(&c).unwrap();
    assert_eq!(flat(&tree), reference_tree(&rows));
    assert_eq!(tree.nodes.capacity(), tree.nodes.len());
    eprintln!("{} folders, {} nodes, {} files", rows.len(), tree.len(), tree.total());
    let sizes = |len: usize, cap: usize| format!("{len} of {cap}");
    eprintln!(
        "paths {}, keys {}, nodes {}",
        sizes(tree.paths.len(), tree.paths.capacity()),
        sizes(tree.keys.len(), tree.keys.capacity()),
        sizes(tree.nodes.len(), tree.nodes.capacity())
    );
}

#[test]
fn name_order() {
    use std::cmp::Ordering::*;
    let cmp = |a: &str, b: &str| name_key(a).cmp(&name_key(b));
    // Case-insensitively first (Unicode, unlike SQLite's NOCASE), then exactly, so the order is total.
    assert_eq!(cmp("a.jpg", "B.jpg"), Less);
    assert_eq!(cmp("Ä.jpg", "b.jpg"), Greater);
    assert_eq!(cmp("ä.jpg", "Ä.JPG"), Greater);
    assert_eq!(cmp("B.jpg", "b.jpg"), Less);
    assert_eq!(cmp("img10", "IMG10"), Greater);
    assert_eq!(cmp("abc", "ABC"), Greater);
    assert_eq!(cmp("ab", "abc"), Less);
    assert_eq!(cmp("same", "same"), Equal);
}

/// The order before the key (kept as the reference): folders by their lowercased merged path component by
/// component (the scope's sort), then names by the PATH collation (characters lowercased one by one, then
/// bytes). Empty components are dropped, as the tree did.
fn reference_order(a: (&str, &str), b: (&str, &str)) -> std::cmp::Ordering {
    let folder = |p: &str| -> Vec<String> {
        merged_path(p).to_lowercase().split('\\').filter(|c| !c.is_empty()).map(String::from).collect()
    };
    let collation = |a: &str, b: &str| {
        a.chars().flat_map(char::to_lowercase).cmp(b.chars().flat_map(char::to_lowercase)).then_with(|| a.cmp(b))
    };
    folder(a.0).cmp(&folder(b.0)).then_with(|| collation(a.1, b.1))
}

/// A small xorshift: the same cases on every run, without a dependency.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn pick<'a>(&mut self, from: &[&'a str]) -> &'a str {
        from[(self.next() % from.len() as u64) as usize]
    }
}

/// Paths and names made of the pieces where lowercasing and byte order are tricky.
fn random_file(rng: &mut Rng) -> (String, String) {
    const PIECES: &[&str] = &[
        "a", "A", "b", "B", "z", "ä", "Ä", "ß", "İ", "i", "\u{212A}", "k", "K", "Σ", "σ", "ς", "e\u{301}", "é", "-",
        ".", " ", "1", "10", "\u{1}", "\u{2}", "\u{3}", "ΑΣ", "ας", "日本", "😀",
    ];
    let part = |rng: &mut Rng| -> String { (0..1 + rng.next() % 3).map(|_| rng.pick(PIECES)).collect() };
    let drive = rng.pick(&["D:", "E:", "d:", r"\\srv\share"]);
    let mut folder = drive.to_string();
    for _ in 0..rng.next() % 4 {
        folder.push('\\');
        folder.push_str(&part(rng));
    }
    (folder, part(rng))
}

#[test]
fn order_keys_sort_as_before() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
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
    before((r"D:\ας", "a"), (r"D:\ασ", "a"));
    before((r"D:\a\b", "z"), ("D:\\a\\b\u{1}c", "a"));
    // A final Σ in a folder name lowercases to ς, as the whole path did.
    assert_eq!(order_key_of(r"D:\ΑΣ", "a"), order_key_of(r"E:\ας", "a"));
}

/// Folder keys as they were made, each component through `str::to_lowercase`: stored keys don't change.
#[test]
fn folder_keys_are_made_as_before() {
    let reference = |merged: &str| -> Vec<u8> {
        let mut key = Vec::new();
        for part in components(merged) {
            for b in part.to_lowercase().bytes() {
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
    let mut rng = Rng(0x2545_F491_4F6C_DD1D);
    let random = (0..400).map(|_| random_file(&mut rng).0);
    for folder in merged.into_iter().map(String::from).chain(random) {
        assert_eq!(folder_key(merged_path(&folder)), reference(merged_path(&folder)), "{folder:?}");
    }
    for name in ["a.JPG", "ΑΣ.jpg", "\u{130}.png", ""] {
        // Made at its exact size (`Vec::with_capacity` gives exactly what it is asked for).
        let key = order_key(b"k\x01", name);
        assert_eq!(key, [&b"k\x01"[..], &name_key(name)].concat());
        assert_eq!(key.capacity(), key.len());
    }
}

/// On a real catalog's names (read-only, any layout): `PG_TEST_CATALOG=… cargo test -p pg-catalog real_names --
/// --ignored`.
#[test]
#[ignore = "needs PG_TEST_CATALOG, a real catalog (opened read-only)"]
fn real_names_sort_and_scope_as_before() {
    let path = std::env::var_os("PG_TEST_CATALOG").expect("PG_TEST_CATALOG");
    let c = crate::Catalog::open_current(std::path::Path::new(&path)).unwrap();
    let conn = &c.conn;
    let files: Vec<(String, String)> = conn
        .prepare("SELECT fo.path, f.name FROM files f JOIN folders fo ON fo.id = f.folder_id")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    let mut by_key = files.clone();
    by_key.sort_by_cached_key(|(folder, name)| order_key_of(folder, name));
    for pair in by_key.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        assert_ne!(reference_order((&a.0, &a.1), (&b.0, &b.1)), std::cmp::Ordering::Greater, "{a:?}, {b:?}");
    }
    // Every folder against every scope, recursive and not: the old match (`in_scope`) and the key range.
    let folders: Vec<String> = conn
        .prepare("SELECT path FROM folders")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    let in_range = |scope_of: (&str, bool), folder: &str| {
        let (_, values) = scope(scope_of.0, scope_of.1);
        let key = order_key_of(folder, "a");
        let blob = |v: &Value| match v {
            Value::Blob(b) => b.clone(),
            _ => unreachable!(),
        };
        key >= blob(&values[0]) && values.get(1).is_none_or(|hi| key < blob(hi))
    };
    for scope_folder in &folders {
        let merged = merged_path(scope_folder).to_lowercase();
        for folder in &folders {
            let m = merged_path(folder).to_lowercase();
            for recursive in [false, true] {
                let before = m == merged
                    || (recursive
                        && (merged.is_empty() || m.strip_prefix(&merged).is_some_and(|rest| rest.starts_with('\\'))));
                assert_eq!(in_range((&merged, recursive), folder), before, "{folder} in {merged} ({recursive})");
            }
        }
    }
    eprintln!("{} files, {} folders: same order and scopes", files.len(), folders.len());
}

#[test]
fn scopes_are_key_ranges() {
    let inside = |scope_of: (&str, bool), file: (&str, &str)| {
        let (sql, values) = scope(scope_of.0, scope_of.1);
        let key = order_key_of(file.0, file.1);
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
#[test]
fn lists_are_made_at_their_size() {
    let c = mixed_library();
    let all = list_grid(&c, "", true, &Query::default(), Sort::default()).unwrap();
    assert_eq!(all.capacity(), all.len());
    let sorted = list_grid(&c, "", true, &Query::default(), by(SortKey::Size, true)).unwrap();
    assert_eq!(sorted.capacity(), sorted.len());
    let one = list_grid(&c, "", true, &Query { ratings: vec![5], ..Query::default() }, Sort::default()).unwrap();
    assert!(one.capacity() < all.len() * 3 / 4 || one.capacity() == one.len());
}

#[test]
fn lists_read_the_order_index() {
    let c = library();
    for (folder, recursive) in [("p", true), ("p", false), ("", true)] {
        for sort in [Sort::default(), by(SortKey::Name, true), by(SortKey::Taken, true)] {
            let (scope, values) = scope(folder, recursive);
            let unrated = Query { ratings: vec![0], ..Query::default() }.condition();
            let plan: Vec<String> = c
                .conn
                .prepare(&format!("EXPLAIN QUERY PLAN {}", grid_sql(&scope, &unrated.sql, sort)))
                .unwrap()
                .query_map(params_from_iter(values.into_iter().chain(unrated.params)), |r| r.get(3))
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap();
            let plan = plan.join("\n");
            assert!(plan.contains("USING INDEX files_order"), "{plan}");
            assert!(!plan.contains("TEMP B-TREE"), "sorted: {plan}");
        }
        // The list's size, from the index alone.
        let (scope, values) = scope(folder, recursive);
        let plan: String = c
            .conn
            .query_row(
                &format!("EXPLAIN QUERY PLAN SELECT count(*) FROM files f WHERE f.missing = 0 AND {scope}"),
                params_from_iter(values),
                |r| r.get(3),
            )
            .unwrap();
        assert!(plan.contains("COVERING INDEX files_order"), "{plan}");
    }
    let plan: Vec<String> = c
        .conn
        .prepare(&format!("EXPLAIN QUERY PLAN {FOLDER_COUNTS}"))
        .unwrap()
        .query_map([], |r| r.get(3))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    let plan = plan.join("\n");
    assert!(plan.contains("COVERING INDEX files_present") && !plan.contains("TEMP B-TREE"), "{plan}");
}

#[test]
fn facets_filter() {
    let c = mixed_library();
    let q = |f: &dyn Fn(&mut Query)| {
        let mut q = Query::default();
        f(&mut q);
        ids(&c, "", true, &q, Sort::default())
    };
    assert_eq!(q(&|_| {}), [2, 6, 1, 7, 8, 3, 4, 5]);
    assert_eq!(q(&|q| q.types = vec![MediaType::Video]), [5]);
    assert_eq!(q(&|q| q.types = vec![MediaType::Animated, MediaType::Video]), [6, 5]);
    assert_eq!(q(&|q| q.types = vec![MediaType::Image]), [2, 1, 7, 8, 3, 4]);
    assert_eq!(q(&|q| q.exts = vec!["JPG".into()]), [1, 7, 3]);
    assert_eq!(q(&|q| q.orientations = vec![Orientation::Square]), [6]);
    assert_eq!(q(&|q| q.orientations = vec![Orientation::Landscape]), [2, 1, 3, 4, 5]);
    assert!(q(&|q| q.orientations = vec![Orientation::Portrait]).is_empty());
    assert_eq!(q(&|q| q.megapixels.min = Some(5e-6)), [2, 6, 1, 3, 4, 5]);
    assert_eq!(q(&|q| q.megapixels.max = Some(1e-5)), [6]);
    assert_eq!(q(&|q| q.size_mb = Range { min: Some(20.0 / 1_048_576.0), max: Some(40.0 / 1_048_576.0) }), [1, 3, 4]);
    assert_eq!(q(&|q| q.duration_s.min = Some(10.0)), [5]);
    assert_eq!(q(&|q| q.duplicates = Some(true)), [2, 1, 3, 4]);
    assert_eq!(q(&|q| q.duplicates = Some(false)), [6, 5]);
    assert_eq!(q(&|q| q.status = vec![Status::Failed]), [7]);
    assert_eq!(q(&|q| q.status = vec![Status::Pending]), [8]);
    assert_eq!(q(&|q| q.status = vec![Status::Imported]), [2, 6, 1, 3, 4, 5]);
    let day = |from: &str, to: &str| DateRange { from: Some(from.into()), to: Some(to.into()) };
    assert_eq!(q(&|q| q.taken = day("2024-05-01", "2024-05-01")), [1]);
    assert!(q(&|q| q.taken.from = Some("2024-05-02".into())).is_empty());
    // A malformed date doesn't filter.
    assert_eq!(q(&|q| q.taken.from = Some("2024-13-45".into())).len(), 8);
    // The test files' mtimes are FILETIMEs a few hundred ns after 1601.
    assert_eq!(q(&|q| q.modified = day("1600-12-30", "1601-01-02")).len(), 8);
    assert!(q(&|q| q.modified.from = Some("2000-01-01".into())).is_empty());
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

#[test]
fn sorts_put_missing_values_last() {
    let c = mixed_library();
    let all = Query::default();
    assert_eq!(ids(&c, "", true, &all, by(SortKey::Megapixels, true)), [5, 2, 1, 3, 4, 6, 7, 8]);
    assert_eq!(ids(&c, "", true, &all, by(SortKey::Megapixels, false)), [6, 2, 1, 3, 4, 5, 7, 8]);
    assert_eq!(ids(&c, "", true, &all, by(SortKey::Duration, true)), [5, 2, 6, 1, 7, 8, 3, 4]);
    assert_eq!(ids(&c, "", true, &all, by(SortKey::Taken, true))[0], 1);
}

#[test]
fn grid_lists_sort_as_sqlite_did() {
    let c = mixed_library();
    // Ties: four 4×3 images, and no taken date or duration on most.
    for key in [SortKey::Name, SortKey::Size, SortKey::Modified, SortKey::Taken, SortKey::Megapixels, SortKey::Duration]
    {
        for descending in [false, true] {
            let sort = by(key, descending);
            assert_eq!(ids(&c, "", true, &Query::default(), sort), sorted_by_sqlite(&c, sort), "{sort:?}");
        }
    }
    // Each entry as read alone.
    for e in list_grid(&c, "", true, &Query::default(), Sort::default()).unwrap() {
        assert_eq!(entry(&c, e.id).unwrap(), Some(e));
    }
    let entries = list_grid(&c, "", true, &Query::default(), Sort::default()).unwrap();
    let get = |id: i64| *entries.iter().find(|e| e.id == FileId::new(id)).unwrap();
    let (clip, gif, broken) = (get(5), get(6), get(7));
    assert_eq!((clip.media_type(), clip.duration_ms(), clip.ext.name()), (MediaType::Video, Some(12000), "mp4"));
    assert_eq!((gif.media_type(), gif.duration_ms(), gif.ext.name()), (MediaType::Animated, None, "gif"));
    // Not imported (its analysis failed): no size yet, square.
    assert_eq!((broken.shown_size(), broken.aspect()), ((0, 0), 1.0));
    assert_eq!(entry(&c, FileId::new(99)).unwrap(), None);
}

#[test]
fn facet_counts() {
    let c = mixed_library();
    let videos = Query { types: vec![MediaType::Video], ..Query::default() };
    let f = facets(&c, "", true, &videos).unwrap();
    assert_eq!((f.total, f.matching), (8, 1));
    // A facet's own choices are counted without it; the other facets apply.
    assert_eq!(f.types, [(MediaType::Image, 6), (MediaType::Animated, 1), (MediaType::Video, 1)]);
    assert_eq!(f.exts, [("mp4".to_string(), 1)]);
    let f = facets(&c, "p", false, &Query::default()).unwrap();
    assert_eq!(
        f.exts,
        [("jpg".to_string(), 3), ("gif".to_string(), 1), ("png".to_string(), 1), ("webp".to_string(), 1)]
    );
}
