//! The catalogs the browse tests read: a small library, and one with a video, an animated GIF, a file the import
//! could not read and one not imported yet.

use pg_model::ids::{FileId, FolderId};
use pg_model::rating::Rating;
use rusqlite::params;

use crate::browse::read::{FILE_JOINS, list_grid};
use crate::fixture;
use crate::query::{Query, Sort, SortKey};
use crate::writes::set_rating;

/// Files on two drives under the same merged folder, plus a subfolder: (id, folder, name, size, mtime,
/// format, rating).
pub(crate) fn library() -> crate::Catalog {
    let c = crate::Catalog::prepare(std::path::Path::new(":memory:"), crate::fixture::versions()).unwrap();
    c.conn
        .execute_batch(
            "INSERT INTO scans(id, root, started) VALUES (1, 'D:', 0);
             INSERT INTO folders(id, path) VALUES (1, 'D:\\p'), (2, 'E:\\p'), (3, 'D:\\p\\sub');",
        )
        .unwrap();
    for (id, folder, name, size, mtime, format, rating) in [
        (1, 1, "b.jpg", 30, 3, "jpeg", 4),
        (2, 2, "a.png", 10, 9, "png", 0),
        (3, 1, "Ä.jpg", 20, 1, "jpeg", 2),
        (4, 3, "c.bmp", 40, 5, "bmp", 0),
    ] {
        fixture::insert(&c, &[fixture::File::new(FileId::new(id), FolderId::new(folder), name, size, mtime)]);
        c.conn
            .execute(
                "INSERT INTO analysis(file_id, version, size, mtime, rotation, kind, analyzed, format, width, height)
                 VALUES (?1, 1, ?2, ?3, 0, 'image', 0, ?4, 4, 3)",
                params![id, size, mtime, format],
            )
            .unwrap();
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
    c.conn
        .execute_batch(
            "INSERT INTO analysis(file_id, version, size, mtime, rotation, kind, analyzed, format, width, height,
                                  frames, duration_ms) VALUES
                (5, 1, 50, 7, 0, 'video', 0, NULL, 1920, 1080, 300, 12000),
                (6, 1, 15, 2, 0, 'image', 0, 'gif', 3, 3, 10, NULL);
             INSERT INTO analysis(file_id, version, size, mtime, rotation, kind, analyzed, error)
                 VALUES (7, 1, 5, 4, 0, 'image', 0, 'bad');
             UPDATE analysis SET file_hash = x'01' WHERE file_id IN (1, 3);
             UPDATE analysis SET taken = 1714557600 WHERE file_id = 1; -- 2024-05-01 10:00
             INSERT INTO pdq_pairs(a, b, distance, variant) VALUES (2, 4, 10, 0), (2, 6, 30, 0);",
        )
        .unwrap();
    c
}

/// The order SQLite gave when it sorted (before the sort moved here): the key, files without it last, then
/// name order ascending.
pub(crate) fn sorted_by_sqlite(c: &crate::Catalog, sort: Sort) -> Vec<i64> {
    let order = match sort.key_sql() {
        None if sort.descending => "f.order_key DESC, f.id DESC".to_string(),
        None => "f.order_key, f.id".to_string(),
        Some(k) => format!("{k} IS NULL, {k} {}, f.order_key, f.id", if sort.descending { "DESC" } else { "ASC" }),
    };
    let sql = format!("SELECT f.id FROM files f {FILE_JOINS} WHERE f.missing = 0 ORDER BY {order}");
    c.conn.prepare(&sql).unwrap().query_map([], |r| r.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap()
}
