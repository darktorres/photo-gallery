//! The catalogs the browse tests read: a small library, and one with a video, an animated GIF, a file the import
//! could not read and one not imported yet. Both are made under the path rules the test asks for: their paths are
//! spelled as Windows spells them, and [`spelled`] makes them the rules' own.

use pg_model::ids::{FileId, FolderId};
use pg_model::rating::Rating;
use pg_model::rules::PathRules;

use crate::browse::read::{FILE_JOINS, list_grid};
use crate::fixture::{self, raw};
use crate::query::key_sql;
use crate::writes::set_rating;
use pg_model::query::{Query, Sort, SortKey};

/// Runs a rule-generic test (one that takes its [`PathRules`]) under both rule sets, as two tests: `$windows` under
/// Windows rules and `$posix` under POSIX ones, so a failure names the rules it failed under.
macro_rules! under_both_rules {
    ($test:ident, $windows:ident, $posix:ident) => {
        #[test]
        fn $windows() {
            $test(pg_model::rules::PathRules::Windows);
        }

        #[test]
        fn $posix() {
            $test(pg_model::rules::PathRules::Posix);
        }
    };
}
pub(crate) use under_both_rules;

/// A path spelled as Windows spells it (`D:\p\sub`), under `rules`: POSIX has no drive, so the drive goes and the
/// folders are `/`-separated (`/p/sub`). A test writes its paths once this way, and both rule sets read them.
pub(crate) fn spelled(rules: PathRules, windows: &str) -> String {
    match rules {
        PathRules::Windows => windows.to_string(),
        PathRules::Posix => {
            // A drive letter and its colon go; the rest is the path as it was.
            let bare = windows
                .strip_prefix(|c: char| c.is_ascii_alphabetic())
                .and_then(|rest| rest.strip_prefix(':'))
                .unwrap_or(windows);
            format!("/{}", bare.trim_start_matches(['\\', '/']).replace('\\', "/"))
        }
    }
}

/// Files on two drives under the same merged folder, plus a subfolder: (id, folder, name, size, mtime,
/// format, rating). POSIX rules have one root, so there the second drive's file is in the first drive's folder (the
/// merged folder is the same either way, and every merged result reads alike); the two drives are a Windows fixture.
pub(crate) fn library(rules: PathRules) -> crate::Catalog {
    let c = crate::Catalog::prepare(std::path::Path::new(":memory:"), crate::fixture::versions(), rules).unwrap();
    let second: i64 = if rules == PathRules::Windows { 2 } else { 1 };
    raw::add_scan(&c, 1, &spelled(rules, "D:"));
    raw::add_folder(&c, 1, &spelled(rules, r"D:\p"));
    if rules == PathRules::Windows {
        raw::add_folder(&c, 2, r"E:\p");
    }
    raw::add_folder(&c, 3, &spelled(rules, r"D:\p\sub"));
    for (id, folder, name, size, mtime, format, rating) in [
        (1, 1, "b.jpg", 30, 3, "jpeg", 4),
        (2, second, "a.png", 10, 9, "png", 0),
        (3, 1, "Ä.jpg", 20, 1, "jpeg", 2),
        (4, 3, "c.bmp", 40, 5, "bmp", 0),
    ] {
        fixture::insert(&c, &[fixture::File::new(FileId::new(id), FolderId::new(folder), name, size, mtime)]).unwrap();
        let row = raw::AnalysisRow {
            format: Some(format),
            width: Some(4),
            height: Some(3),
            ..raw::AnalysisRow::image(id, size, mtime)
        };
        raw::insert_analysis(&c, &row);
        set_rating(&c, FileId::new(id), Rating::new(rating).filter(|r| r.get() > 0), false).unwrap();
    }
    c
}

/// The ids of a folder's list, the folder given as its path text (`""` for all folders) under the catalog's rules.
pub(crate) fn ids(c: &crate::Catalog, folder: &str, recursive: bool, query: &Query, sort: Sort) -> Vec<i64> {
    let folder = folder_of(c, folder);
    list_grid(c, &folder, recursive, query, sort).unwrap().iter().map(|e| e.id.get()).collect()
}

/// A folder given as its path text (`""` for all folders), merged under the catalog's rules.
pub(crate) fn folder_of(c: &crate::Catalog, text: &str) -> pg_model::rules::Merged {
    let rules = c.rules();
    rules.merge(&rules.parse(text).unwrap())
}

pub(crate) fn by(key: SortKey, descending: bool) -> Sort {
    Sort { key, descending }
}

/// `library(rules)` plus a video, an animated GIF, a file the import couldn't read and one not imported yet, with
/// duplicates: 1 and 3 are the same bytes, 2–4 a close tier-1 pair, 2–6 a distant one.
pub(crate) fn mixed_library(rules: PathRules) -> crate::Catalog {
    let c = library(rules);
    use fixture::File;
    let second = FolderId::new(if rules == PathRules::Windows { 2 } else { 1 });
    fixture::insert(
        &c,
        &[
            File { kind: "video", ..File::new(FileId::new(5), FolderId::new(3), "Clip.MP4", 50, 7) },
            File::new(FileId::new(6), FolderId::new(1), "anim.gif", 15, 2),
            File::new(FileId::new(7), second, "broken.jpg", 5, 4),
            File::new(FileId::new(8), second, "new.webp", 6, 6),
        ],
    )
    .unwrap();
    let video = raw::AnalysisRow {
        kind: "video",
        width: Some(1920),
        height: Some(1080),
        frames: Some(300),
        duration_ms: Some(12000),
        ..raw::AnalysisRow::image(5, 50, 7)
    };
    raw::insert_analysis(&c, &video);
    let gif = raw::AnalysisRow {
        format: Some("gif"),
        width: Some(3),
        height: Some(3),
        frames: Some(10),
        ..raw::AnalysisRow::image(6, 15, 2)
    };
    raw::insert_analysis(&c, &gif);
    raw::insert_analysis(&c, &raw::AnalysisRow { error: Some("bad"), ..raw::AnalysisRow::image(7, 5, 4) });
    raw::set_file_hash(&c, &[1, 3], &[1]);
    raw::set_taken(&c, 1, 1714557600); // 2024-05-01 10:00
    raw::add_pdq_pair(&c, 2, 4, 10);
    raw::add_pdq_pair(&c, 2, 6, 30);
    c
}

/// The order SQLite gave when it sorted (before the sort moved here): the key, files without it last, then
/// name order ascending.
pub(crate) fn sorted_by_sqlite(c: &crate::Catalog, sort: Sort) -> Vec<i64> {
    let order = match key_sql(sort) {
        None if sort.descending => "f.order_key DESC, f.id DESC".to_string(),
        None => "f.order_key, f.id".to_string(),
        Some(k) => format!("{k} IS NULL, {k} {}, f.order_key, f.id", if sort.descending { "DESC" } else { "ASC" }),
    };
    let sql = format!("SELECT f.id FROM files f {FILE_JOINS} WHERE f.missing = 0 ORDER BY {order}");
    raw::rows_of(c, &sql, [], |r| r.get(0))
}
