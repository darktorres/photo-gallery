//! The catalogs the browse tests read: a small library, and one with a video, an animated GIF, a file the import
//! could not read and one not imported yet.

use pg_model::ids::{FileId, FolderId};
use pg_model::rating::Rating;

use crate::browse::read::{FILE_JOINS, list_grid};
use crate::fixture::{self, raw};
use crate::query::key_sql;
use crate::writes::set_rating;
use pg_model::query::{Query, Sort, SortKey};

/// Files on two drives under the same merged folder, plus a subfolder: (id, folder, name, size, mtime,
/// format, rating).
pub(crate) fn library() -> crate::Catalog {
    let c = crate::Catalog::prepare(std::path::Path::new(":memory:"), crate::fixture::versions()).unwrap();
    raw::add_scan(&c, 1, "D:");
    raw::add_folder(&c, 1, r"D:\p");
    raw::add_folder(&c, 2, r"E:\p");
    raw::add_folder(&c, 3, r"D:\p\sub");
    for (id, folder, name, size, mtime, format, rating) in [
        (1, 1, "b.jpg", 30, 3, "jpeg", 4),
        (2, 2, "a.png", 10, 9, "png", 0),
        (3, 1, "Ä.jpg", 20, 1, "jpeg", 2),
        (4, 3, "c.bmp", 40, 5, "bmp", 0),
    ] {
        fixture::insert(&c, &[fixture::File::new(FileId::new(id), FolderId::new(folder), name, size, mtime)]);
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

pub(crate) fn ids(c: &crate::Catalog, folder: &str, recursive: bool, query: &Query, sort: Sort) -> Vec<i64> {
    list_grid(c, folder, recursive, query, sort).unwrap().iter().map(|e| e.id.get()).collect()
}

pub(crate) fn by(key: SortKey, descending: bool) -> Sort {
    Sort { key, descending }
}

/// `library()` plus a video, an animated GIF, a file the import couldn't read and one not imported yet, with
/// duplicates: 1 and 3 are the same bytes, 2–4 a close tier-1 pair, 2–6 a distant one.
pub(crate) fn mixed_library() -> crate::Catalog {
    let c = library();
    use fixture::File;
    fixture::insert(
        &c,
        &[
            File { kind: "video", ..File::new(FileId::new(5), FolderId::new(3), "Clip.MP4", 50, 7) },
            File::new(FileId::new(6), FolderId::new(1), "anim.gif", 15, 2),
            File::new(FileId::new(7), FolderId::new(2), "broken.jpg", 5, 4),
            File::new(FileId::new(8), FolderId::new(2), "new.webp", 6, 6),
        ],
    );
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
