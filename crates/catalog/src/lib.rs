//! The catalog (PLAN §6): SQLite in WAL mode, the source of truth for files, analyses, pairs and ratings.
//!
//! File identity is (volume serial, NTFS file ID), so moves and renames need no re-analysis (PLAN §4). An analysis
//! row records what it was computed from: the size, mtime, catalog rotation and kind, and the pipeline version.
//! When any differ, the file is pending again (`files.pending`, kept by triggers). That is all the resumability
//! there is: every batch is one transaction, and the work list is an index.
//!
//! The thumbnails' work is tracked the same way: whatever a thumbnail depends on changing gives the file the next
//! number of a change sequence (`files.seq`, from `meta.seq`), so the thumbnail pass looks only at files changed
//! since it last looked (`thumbs::pending_in`).
// A library reports through its results; printing is for the command line and the app.
#![cfg_attr(not(test), deny(clippy::print_stdout, clippy::print_stderr))]

// The SQL macros are declared first: `macro_rules!` is textually scoped, so every module declared after
// this one can use them.
#[macro_use]
mod sql;

pub mod browse;
pub mod dups;
#[cfg(any(test, feature = "fixture"))]
pub mod fixture;
pub mod writes;

mod core;
mod lifecycle;
mod types;
mod writers;

mod error;
pub use error::{Error, Result};
mod pair;
pub use pair::{Coverage, Pairing};
mod schema;

pub use self::core::{Catalog, Tx};
pub use lifecycle::max_folder_id;
pub use types::{Carried, CropSource, Folder, Rebuilt, Recorded, Root, ScannedFile, Status, Versions, WorkItem};
pub use writers::{AnalysisWriter, BordersWriter, ScanWriter};
pub(crate) use types::{DELETE_FILE_RATING, SET_FILE_RATING, folders_under, rules_name, skipped_folders, under_params};
#[cfg(any(test, feature = "fixture"))]
pub(crate) use lifecycle::{set_borders_version, set_import_version};

// What the tests use through `use super::*` and the modules above do not import for themselves.
#[cfg(test)]
use lifecycle::{pending_import_sql, set_taken_zone};
#[cfg(test)]
use schema::{BASELINE, MIGRATIONS, bring_to, current_expected, current_version, mismatch};
#[cfg(test)]
use types::TAKEN_ZONE_QUERY;
#[cfg(test)]
use pg_model::analysis::Analyzed;
#[cfg(test)]
use pg_model::detection::Detection;
#[cfg(test)]
use pg_model::format::Format;
#[cfg(test)]
use pg_model::ids::{FileId, FileIdentity, FolderId, RootId};
#[cfg(test)]
use pg_model::media::{Ext, Kind};
#[cfg(test)]
use pg_model::name::Name;
#[cfg(test)]
use pg_model::orientation::Orientation;
#[cfg(test)]
use pg_model::rating::Rating;
#[cfg(test)]
use pg_model::rules::{Path as RulePath, PathRules};
#[cfg(test)]
use pg_model::turns::Turns;
#[cfg(test)]
use pg_platform::sqlite::sqlite_path;
#[cfg(test)]
use rusqlite::Connection;
#[cfg(test)]
use std::path::Path;
#[cfg(test)]
use std::sync::Arc;
#[cfg(test)]
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browse::fixtures::{spelled, under_both_rules};
    use crate::fixture::raw;
    use pg_analysis::analyze::IMPORT_VERSION;
    use pg_analysis::borders::BORDERS_VERSION;
    use pg_model::analysis::{Analysis, ImageFacts, Media, PixelHashes, Taken, ThumbHash};
    use pg_testkit::test_folder;

    /// The versions this build's analyses are stamped with, as the library passes them to [`Catalog::prepare`].
    fn versions() -> Versions {
        Versions { import: IMPORT_VERSION, borders: BORDERS_VERSION }
    }

    /// A path under `rules`, from its text as the tests write it, in the Windows form (`D:\p\sub`), which [`spelled`]
    /// turns into the form of `rules` (`/p/sub` under POSIX).
    fn p(rules: PathRules, text: &str) -> RulePath {
        rules.parse(&spelled(rules, text)).expect("a test path")
    }

    /// Where this module's tests write (`PG_TEST_DIR` when it is set).
    const SCRATCH: &str = "scratch_catalog";

    /// The layout before the order key (2026-10-31), as the catalogs made with it have it.
    const BEFORE_ORDER_KEY: &str = r"
        PRAGMA journal_mode = WAL;
        CREATE TABLE meta(key TEXT PRIMARY KEY, value ANY NOT NULL);
        INSERT INTO meta(key, value) VALUES ('path_rules', 'windows');
        CREATE TABLE scans(id INTEGER PRIMARY KEY, root TEXT NOT NULL, started INTEGER NOT NULL, finished INTEGER);
        CREATE TABLE folders(id INTEGER PRIMARY KEY, path TEXT NOT NULL UNIQUE);
        CREATE TABLE files(id INTEGER PRIMARY KEY, volume INTEGER NOT NULL, file_ref INTEGER NOT NULL,
            folder_id INTEGER NOT NULL, name TEXT NOT NULL, ext TEXT NOT NULL, kind TEXT NOT NULL,
            size INTEGER NOT NULL, mtime INTEGER NOT NULL, last_scan INTEGER NOT NULL,
            missing INTEGER NOT NULL DEFAULT 0, rotation INTEGER NOT NULL DEFAULT 0, UNIQUE(volume, file_ref));
        CREATE TABLE analysis(file_id INTEGER PRIMARY KEY, version INTEGER NOT NULL, size INTEGER NOT NULL,
            mtime INTEGER NOT NULL, analyzed INTEGER NOT NULL, error TEXT, file_hash BLOB, pixel_hash BLOB,
            format TEXT, orientation INTEGER, width INTEGER, height INTEGER, pdq BLOB, pdq_quality INTEGER,
            thumbhash BLOB, frames INTEGER, taken INTEGER, duration_ms INTEGER, poster_ms INTEGER);
        CREATE TABLE pdq_pairs(a INTEGER NOT NULL, b INTEGER NOT NULL, distance INTEGER NOT NULL,
            variant INTEGER NOT NULL, PRIMARY KEY(a, b)) WITHOUT ROWID;
        CREATE TABLE roots(path TEXT PRIMARY KEY, added INTEGER NOT NULL);
        CREATE TABLE ratings(file_id INTEGER PRIMARY KEY, rating INTEGER NOT NULL, updated INTEGER NOT NULL);
        CREATE TABLE file_ratings(file_id INTEGER PRIMARY KEY, value INTEGER NOT NULL);
        CREATE TABLE file_writes(file_id INTEGER NOT NULL, kind TEXT NOT NULL, value INTEGER NOT NULL,
            queued INTEGER NOT NULL, attempts INTEGER NOT NULL DEFAULT 0, error TEXT, temp TEXT, temp_value INTEGER,
            PRIMARY KEY(file_id, kind));
        CREATE TABLE views(name TEXT PRIMARY KEY, folder TEXT NOT NULL, recursive INTEGER NOT NULL,
            query TEXT NOT NULL, sort TEXT NOT NULL);
        CREATE TABLE session(key TEXT PRIMARY KEY, value TEXT NOT NULL);
        PRAGMA user_version = 1;";

    // The layout tests below read the file on disk through their own connection (`raw::on_disk`): what they check
    // is the layout, not the catalog's tables.

    /// A catalog that isn't there is an error to open, and opening it makes no marker beside where it would be.
    #[test]
    fn opening_a_catalog_that_is_not_there_makes_no_marker() {
        let dir = test_folder(SCRATCH, "not-there").unwrap();
        let path = dir.join("catalog.sqlite");
        assert!(Catalog::open(&path, PathRules::NATIVE).is_err());
        assert!(!dir.join("catalog.sqlite.live").exists(), "no marker for a catalog that isn't there");
    }

    /// A catalog another connection has open is not moved aside: its rebuild is refused, with the file as it was, and it
    /// is rebuilt once that connection has closed. The edit to the file is made on a raw connection, which holds nothing.
    #[test]
    fn a_catalog_another_connection_has_open_is_not_moved_aside() {
        let dir = test_folder(SCRATCH, "open-elsewhere").unwrap();
        let path = dir.join("catalog.sqlite");
        let open = Catalog::prepare(&path, versions(), PathRules::NATIVE).unwrap();
        let edited = Connection::open(&path).unwrap();
        edited.execute_batch("CREATE INDEX extra_names ON files(name);").unwrap();
        drop(edited);

        let err = Catalog::prepare(&path, versions(), PathRules::NATIVE).err().expect("refused");
        assert!(format!("{err:#}").contains("has it open"), "{err:#}");
        assert!(path.exists(), "the catalog is where it was");
        assert!(!dir.join("catalog.sqlite.replaced").exists(), "nothing was moved aside");
        drop(open);

        let c = Catalog::prepare(&path, versions(), PathRules::NATIVE).unwrap();
        assert!(c.rebuilt().is_some(), "moved aside once nothing has it open");
    }

    // Windows-only: the earlier layout is a Windows catalog, its rows are `D:\` paths recorded as `windows`.
    #[test]
    fn an_earlier_layout_is_moved_aside_and_a_new_catalog_is_made() {
        let dir = test_folder(SCRATCH, "move-aside").unwrap();
        let path = dir.join("catalog.sqlite");
        let old = Connection::open(&path).unwrap();
        old.execute_batch(BEFORE_ORDER_KEY).unwrap();
        old.execute_batch(&format!(
            r"INSERT INTO scans VALUES (1, 'D:\p', 10, 20);
              INSERT INTO folders VALUES (1, 'D:\p'), (2, 'D:\p\Sub');
              INSERT INTO files VALUES (11, 7, 101, 1, 'B.jpg', 'jpg', 'image', 100, 5, 1, 0, 0),
                                       (12, 7, 102, 2, 'a.png', 'png', 'image', 200, 6, 1, 0, 1),
                                       (13, 7, 103, 1, 'new.jpg', 'jpg', 'image', 300, 7, 1, 0, 0),
                                       (14, 7, 104, 1, 'changed.jpg', 'jpg', 'image', 400, 8, 1, 0, 0);
              INSERT INTO analysis(file_id, version, size, mtime, analyzed, width, height)
                  VALUES (11, {IMPORT_VERSION}, 100, 5, 30, 640, 480), (12, {IMPORT_VERSION}, 200, 6, 30, 10, 20),
                         (14, {IMPORT_VERSION}, 399, 8, 30, 1, 1);
              INSERT INTO pdq_pairs VALUES (11, 12, 3, 0);
              INSERT INTO roots VALUES ('D:\p', 1);
              INSERT INTO ratings VALUES (11, 4, 40), (13, 1, 41);
              INSERT INTO file_ratings VALUES (11, 4);
              INSERT INTO file_writes(file_id, kind, value, queued) VALUES (12, 'rotation', 1, 50);
              INSERT INTO views VALUES ('best', 'p', 1, '{{}}', '{{}}');
              INSERT INTO session VALUES ('folder', 'p');"
        ))
        .unwrap();
        drop(old);

        let c = Catalog::prepare(&path, versions(), PathRules::Windows).unwrap();
        let disk = raw::on_disk(&path);
        assert!(is_current(&disk).unwrap());
        let count = |conn: &Connection, table: &str| -> i64 {
            conn.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0)).unwrap()
        };
        // The old layout lacks columns the new one needs as NOT NULL (`folders.key`, then `files.order_key`): the copy
        // fails as a whole, so nothing of the old rows is in the new catalog (not even its folders), and the reason is
        // said. The earlier one is kept whole, rows and all, beside it.
        let r = c.rebuilt().expect("moved aside");
        assert!(
            matches!(&r.carried, Carried::Nothing(why) if why.starts_with("NOT NULL constraint failed")),
            "{:?}",
            r.carried
        );
        assert_eq!(count(&disk, "files"), 0);
        assert_eq!(count(&disk, "roots"), 0);
        assert!(c.roots().unwrap().is_empty());
        let moved = raw::on_disk(&dir.join("catalog.sqlite.replaced"));
        assert_eq!(count(&moved, "files"), 4);
        assert_eq!(count(&moved, "roots"), 1);
        drop(moved);
        drop(disk);
        drop(c);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A catalog that differs from this build's schema only by an index it shouldn't have is rebuilt with its rows: the
    /// same files under the same ids, and the library folders.
    fn a_catalog_that_differs_keeps_its_rows(rules: PathRules) {
        let dir = test_folder(SCRATCH, &format!("keeps-rows-{}", rules_name(rules))).unwrap();
        let path = dir.join("catalog.sqlite");
        let c = Catalog::prepare(&path, versions(), rules).unwrap();
        raw::add_scan(&c, 1, &spelled(rules, r"D:\p"));
        raw::add_folder(&c, 1, &spelled(rules, r"D:\p"));
        raw::add_folder(&c, 2, &spelled(rules, r"D:\p\Sub"));
        fixture::insert(
            &c,
            &[
                fixture::File::new(FileId::new(11), FolderId::new(1), "B.jpg", 100, 5),
                fixture::File::new(FileId::new(12), FolderId::new(2), "a.png", 200, 6),
            ],
        )
        .unwrap();
        assert!(c.add_root(&p(rules, r"D:\p")).unwrap());
        drop(c);
        let edited = Connection::open(&path).unwrap();
        edited.execute_batch("CREATE INDEX extra_names ON files(name);").unwrap();
        drop(edited);

        let c = Catalog::prepare(&path, versions(), rules).unwrap();
        let r = c.rebuilt().expect("moved aside");
        assert!(r.why.contains("extra_names"), "{}", r.why);
        assert_eq!(r.carried, Carried::Rows { files: 2, folders: 1 });
        let disk = raw::on_disk(&path);
        assert!(is_current(&disk).unwrap());
        let names: Vec<(i64, String)> = disk
            .prepare("SELECT id, CAST(name AS TEXT) FROM files ORDER BY id")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(names, [(11, "B.jpg".to_string()), (12, "a.png".to_string())]);
        assert_eq!(c.roots().unwrap().len(), 1);
        drop(disk);
        drop(c);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    under_both_rules!(
        a_catalog_that_differs_keeps_its_rows,
        a_catalog_that_differs_keeps_its_rows_windows,
        a_catalog_that_differs_keeps_its_rows_posix
    );

    // Unix: a catalog in a folder whose name has no UTF-8 form is rebuilt with its rows, as the old file is attached
    // by its exact bytes.
    #[cfg(unix)]
    #[test]
    fn a_catalog_in_a_folder_with_a_name_that_is_not_utf8_keeps_its_rows() {
        use std::os::unix::ffi::OsStringExt;
        let base = test_folder(SCRATCH, "odd-folder-name").unwrap();
        let dir = base.join(std::ffi::OsString::from_vec(b"odd-\xFF".to_vec()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("catalog.sqlite");
        let rules = PathRules::Posix;
        let c = Catalog::prepare(&path, versions(), rules).unwrap();
        raw::add_scan(&c, 1, "/p");
        raw::add_folder(&c, 1, "/p");
        fixture::insert(&c, &[fixture::File::new(FileId::new(11), FolderId::new(1), "B.jpg", 100, 5)]).unwrap();
        assert!(c.add_root(&p(rules, "/p")).unwrap());
        drop(c);
        let edited = Connection::open(&path).unwrap();
        edited.execute_batch("CREATE INDEX extra_names ON files(name);").unwrap();
        drop(edited);

        let c = Catalog::prepare(&path, versions(), rules).unwrap();
        let r = c.rebuilt().expect("moved aside");
        assert_eq!(r.carried, Carried::Rows { files: 1, folders: 1 });
    }

    /// A catalog from before the folders were typed kept its saved views' folders and the session's as text. They are
    /// carried as the typed folders (a view's stored bytes, the session's JSON), so they read as they were saved.
    // Windows-only: before the folders were typed, the views and the session held backslash-separated Windows text,
    // and that text is what this carries.
    #[test]
    fn text_folders_of_views_and_session_are_carried_typed() {
        let dir = test_folder(SCRATCH, "text-folders").unwrap();
        let path = dir.join("catalog.sqlite");
        let c = Catalog::prepare(&path, versions(), PathRules::Windows).unwrap();
        crate::browse::writes::session_set_many(&c, &[("folder", r"pinterest\a")]).unwrap();
        drop(c);
        let old = Connection::open(&path).unwrap();
        old.execute_batch(
            r"DROP TABLE views;
              CREATE TABLE views(name TEXT PRIMARY KEY, folder TEXT NOT NULL, recursive INTEGER NOT NULL,
                                 query TEXT NOT NULL, sort TEXT NOT NULL);
              INSERT INTO views VALUES ('old', 'pinterest\a', 1, '{}', '{}');",
        )
        .unwrap();
        drop(old);
        let c = Catalog::prepare(&path, versions(), PathRules::Windows).unwrap();
        let r = c.rebuilt().expect("moved aside");
        assert_eq!(r.carried, Carried::Rows { files: 0, folders: 0 });
        let saved = crate::browse::read::views(&c).unwrap();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].folder, pg_model::rules::Merged::from(vec![Name::from("pinterest"), Name::from("a")]));
        assert_eq!(crate::browse::read::session_get(&c, "folder").unwrap().as_deref(), Some(r#"["pinterest","a"]"#));
        drop(c);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A catalog from before the queued writes' temps were bytes keeps them: a copy that was finished when the app
    /// stopped is still the job it was, with its temp at the same path.
    #[test]
    fn a_queued_temp_from_before_it_was_bytes_is_carried_and_read_back() {
        let dir = test_folder(SCRATCH, "text-temps").unwrap();
        let path = dir.join("catalog.sqlite");
        let c = Catalog::prepare(&path, versions(), PathRules::Windows).unwrap();
        raw::add_scan(&c, 1, r"D:\p");
        raw::add_folder(&c, 1, r"D:\p");
        fixture::insert(&c, &[fixture::File::new(FileId::new(1), FolderId::new(1), "a.jpg", 1, 5)]).unwrap();
        drop(c);
        let old = Connection::open(&path).unwrap();
        old.execute_batch(
            r"DROP TABLE file_writes;
              CREATE TABLE file_writes(
                  file_id  INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
                  kind     TEXT NOT NULL CHECK (kind IN ('rating', 'rotation')),
                  value    INTEGER NOT NULL,
                  queued   INTEGER NOT NULL,
                  attempts INTEGER NOT NULL DEFAULT 0,
                  error    TEXT,
                  temp       TEXT,
                  temp_value INTEGER,
                  PRIMARY KEY(file_id, kind));
              INSERT INTO file_writes(file_id, kind, value, queued, temp, temp_value)
                  VALUES (1, 'rating', 3, 0, 'D:\p\.a.jpg.pg-write', 3);",
        )
        .unwrap();
        drop(old);
        let c = Catalog::prepare(&path, versions(), PathRules::Windows).unwrap();
        assert!(c.rebuilt().is_some(), "moved aside");
        let job = crate::writes::next_ready(&c, 0).unwrap().expect("the queued copy");
        let temp = PathRules::Windows.parse(r"D:\p\.a.jpg.pg-write").unwrap();
        assert_eq!(job.temp, Some((temp, 3)));
        drop(c);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// On Windows, a catalog in a folder whose name has no UTF-8 form is opened, and rebuilt with its rows carried (the
    /// carry attaches the old file by name): every SQLite call goes through the folder's alias (`pg_platform::sqlite`).
    #[cfg(windows)]
    #[test]
    fn a_catalog_in_a_folder_without_a_utf8_name_is_opened_and_carried_through_an_alias() {
        let dir = test_folder(SCRATCH, "odd-folder").unwrap();
        let folder = dir.join(pg_platform::names::path_buf(b"cat\xED\xA0\x80"));
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join("catalog.sqlite");
        drop(Catalog::prepare(&path, versions(), PathRules::NATIVE).unwrap());
        drop(Catalog::open(&path, PathRules::NATIVE).unwrap());
        // The layout of before the queued temps were bytes: the rebuild carries the old file's rows, attached by name.
        let old = Connection::open(sqlite_path(&path).unwrap()).unwrap();
        old.execute_batch(
            r"DROP TABLE file_writes;
              CREATE TABLE file_writes(
                  file_id  INTEGER NOT NULL REFERENCES files(id) ON DELETE CASCADE,
                  kind     TEXT NOT NULL CHECK (kind IN ('rating', 'rotation')),
                  value    INTEGER NOT NULL,
                  queued   INTEGER NOT NULL,
                  attempts INTEGER NOT NULL DEFAULT 0,
                  error    TEXT,
                  temp       TEXT,
                  temp_value INTEGER,
                  PRIMARY KEY(file_id, kind));",
        )
        .unwrap();
        drop(old);
        let c = Catalog::prepare(&path, versions(), PathRules::NATIVE).unwrap();
        assert!(c.rebuilt().is_some(), "moved aside, and its rows carried");
        drop(c);
        assert!(folder.join("catalog.sqlite").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Rows stored under other rules are not carried into a catalog of these: their paths don't read as its own.
    #[test]
    fn rows_of_other_rules_are_not_carried_over() {
        let dir = test_folder(SCRATCH, "folders-rules").unwrap();
        let path = dir.join("catalog.sqlite");
        let old = Connection::open(&path).unwrap();
        old.execute_batch(BEFORE_ORDER_KEY).unwrap();
        old.execute_batch(r"INSERT INTO roots VALUES ('D:\p', 1);").unwrap();
        drop(old);
        let c = Catalog::prepare(&path, versions(), PathRules::Posix).unwrap();
        let r = c.rebuilt().expect("moved aside");
        assert_eq!(r.carried, Carried::Nothing("made with the windows path rules".to_string()));
        assert!(c.roots().unwrap().is_empty());
        drop(c);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn a_catalog_with_the_earlier_source_trigger_or_view_is_moved_aside(rules: PathRules) {
        let dir = test_folder(SCRATCH, &format!("layout_source-{}", rules_name(rules))).unwrap();
        let path = dir.join("catalog.sqlite");
        drop(Catalog::prepare(&path, versions(), rules).unwrap());
        let fresh = raw::on_disk(&path);
        assert!(is_current(&fresh).unwrap());
        // The same names and counts, the text before the size was added: the layout check must refuse it.
        let text = |name: &str| -> String {
            fresh.query_row("SELECT sql FROM sqlite_schema WHERE name = ?1", [name], |r| r.get(0)).unwrap()
        };
        let (trigger, view) = (text("files_source_changed"), text("thumb_sources"));
        let old_trigger = trigger.replace(", size ON files", " ON files").replace(
            "
    OR old.size IS NOT new.size",
            "",
        );
        let old_view = view.replace("f.size, f.missing", "f.missing");
        assert!(old_trigger != trigger && old_view != view);
        fresh
            .execute_batch(&format!(
                "DROP TRIGGER files_source_changed; {old_trigger}; DROP VIEW thumb_sources; {old_view};"
            ))
            .unwrap();
        drop(fresh);
        let earlier = raw::on_disk(&path);
        assert!(!is_current(&earlier).unwrap());
        drop(earlier);

        let c = Catalog::prepare(&path, versions(), rules).unwrap();
        let disk = raw::on_disk(&path);
        assert!(is_current(&disk).unwrap());
        let current: String = disk
            .query_row(
                "SELECT group_concat(sql, ' ') FROM sqlite_schema WHERE name IN ('files_source_changed', 'thumb_sources')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(current.contains("old.size IS NOT new.size") && current.contains("f.size, f.missing"), "{current}");
        assert!(dir.join("catalog.sqlite.replaced").exists());
        drop(disk);
        drop(c);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    under_both_rules!(
        a_catalog_with_the_earlier_source_trigger_or_view_is_moved_aside,
        a_catalog_with_the_earlier_source_trigger_or_view_is_moved_aside_windows,
        a_catalog_with_the_earlier_source_trigger_or_view_is_moved_aside_posix
    );

    fn a_catalog_of_an_unknown_layout_is_kept_whole_and_a_second_one_takes_a_new_name(rules: PathRules) {
        let dir = test_folder(SCRATCH, &format!("move-aside-twice-{}", rules_name(rules))).unwrap();
        let path = dir.join("catalog.sqlite");
        // Files without the columns the current layout needs.
        let old_layout = |path: &Path| {
            let conn = Connection::open(path).unwrap();
            conn.execute_batch(&format!(
                "CREATE TABLE meta(key TEXT PRIMARY KEY, value ANY NOT NULL); INSERT INTO meta VALUES ('path_rules', '{}');
                 CREATE TABLE files(id INTEGER PRIMARY KEY, name TEXT); INSERT INTO files VALUES (1, 'x');
                 PRAGMA user_version = 1;",
                rules_name(rules)
            ))
            .unwrap();
        };
        old_layout(&path);
        drop(Catalog::prepare(&path, versions(), rules).unwrap());
        let moved = raw::on_disk(&dir.join("catalog.sqlite.replaced"));
        assert_eq!(moved.query_row("SELECT name FROM files", [], |r| r.get::<_, String>(0)).unwrap(), "x");
        drop(moved);
        // Another one moved aside later keeps the first: the new catalog made in the first one's place goes first.
        for part in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(dir.join(format!("catalog.sqlite{part}")));
        }
        old_layout(&path);
        drop(Catalog::prepare(&path, versions(), rules).unwrap());
        assert!(dir.join("catalog.sqlite.replaced.1").exists());
        assert!(dir.join("catalog.sqlite.replaced").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    under_both_rules!(
        a_catalog_of_an_unknown_layout_is_kept_whole_and_a_second_one_takes_a_new_name,
        a_catalog_of_an_unknown_layout_is_kept_whole_and_a_second_one_takes_a_new_name_windows,
        a_catalog_of_an_unknown_layout_is_kept_whole_and_a_second_one_takes_a_new_name_posix
    );

    fn an_earlier_layout_with_no_rules_recorded_is_rebuilt_not_refused(rules: PathRules) {
        let dir = test_folder(SCRATCH, &format!("move-aside-no-rules-{}", rules_name(rules))).unwrap();
        let path = dir.join("catalog.sqlite");
        // An older build's catalog: no `path_rules` row, and the files without the current layout's columns.
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE meta(key TEXT PRIMARY KEY, value ANY NOT NULL);
                 CREATE TABLE files(id INTEGER PRIMARY KEY, name TEXT); INSERT INTO files VALUES (1, 'x');
                 PRAGMA user_version = 1;",
            )
            .unwrap();
        }
        let c = Catalog::prepare(&path, versions(), rules).expect("rebuilt, not refused");
        drop(c);
        let moved = raw::on_disk(&dir.join("catalog.sqlite.replaced"));
        assert_eq!(moved.query_row("SELECT name FROM files", [], |r| r.get::<_, String>(0)).unwrap(), "x");
        drop(moved);
        // The new catalog records its rules, so it opens again.
        drop(Catalog::prepare(&path, versions(), rules).unwrap());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    under_both_rules!(
        an_earlier_layout_with_no_rules_recorded_is_rebuilt_not_refused,
        an_earlier_layout_with_no_rules_recorded_is_rebuilt_not_refused_windows,
        an_earlier_layout_with_no_rules_recorded_is_rebuilt_not_refused_posix
    );

    fn catalogs_have_their_own_id(rules: PathRules) {
        let (a, b) = (
            Catalog::prepare(Path::new(":memory:"), versions(), rules).unwrap(),
            Catalog::prepare(Path::new(":memory:"), versions(), rules).unwrap(),
        );
        assert_eq!(a.catalog_id().unwrap().len(), 32);
        assert_ne!(a.catalog_id().unwrap(), b.catalog_id().unwrap());
    }

    under_both_rules!(catalogs_have_their_own_id, catalogs_have_their_own_id_windows, catalogs_have_their_own_id_posix);

    #[test]
    fn a_catalog_made_under_other_rules_is_refused() {
        let dir = test_folder(SCRATCH, "path-rules").unwrap();
        let path = dir.join("catalog.sqlite");
        drop(Catalog::prepare(&path, versions(), PathRules::Windows).unwrap());
        // Another build's rules: its paths would read as other paths. The catalog is valid, so it is refused, not moved.
        let e = Catalog::open(&path, PathRules::Posix).err().expect("refused");
        assert!(matches!(e, Error::PathRules { .. }), "{e:?}");
        assert!(e.to_string().contains("on the platform that made it"), "{e}");
        assert!(matches!(Catalog::prepare(&path, versions(), PathRules::Posix), Err(Error::PathRules { .. })));
        assert!(!dir.join("catalog.sqlite.replaced").exists());
        // Its rules row gone is not this build's schema: it is moved aside, and the new catalog is made.
        raw::on_disk(&path).execute("DELETE FROM meta WHERE key = 'path_rules'", []).unwrap();
        assert!(Catalog::open(&path, PathRules::Windows).is_err());
        let c = Catalog::prepare(&path, versions(), PathRules::Windows).unwrap();
        assert!(c.rebuilt().is_some());
        drop(c);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn posix_paths_are_bytes_under_case_sensitive_rules() {
        let c = Catalog::prepare(Path::new(":memory:"), versions(), PathRules::Posix).unwrap();
        let rules = c.rules();
        raw::add_scan(&c, 1, "/photos");
        raw::add_folder(&c, 1, "/photos");
        raw::add_folder(&c, 2, "/photos/Pics");
        raw::add_folder(&c, 3, "/photos/pics");
        fixture::insert(
            &c,
            &[
                fixture::File::new(FileId::new(1), FolderId::new(1), "a.jpg", 10, 100),
                fixture::File::new(FileId::new(2), FolderId::new(2), "b.jpg", 10, 100),
                fixture::File::new(FileId::new(3), FolderId::new(3), "b.jpg", 10, 100),
            ],
        )
        .unwrap();
        let root = |text: &str| rules.parse(text).expect("a test path");
        assert!(c.add_root(&root("/photos")).unwrap());
        assert!(!c.add_root(&root("/photos/Pics")).unwrap(), "inside a root");
        assert!(c.add_root(&root("/Photos/x")).unwrap(), "case is significant on POSIX");
        assert_eq!(rules.display(&c.file_path(FileId::new(2)).unwrap().unwrap()), "/photos/Pics/b.jpg");
        // Folders that differ only in case are two folders, and their files are two files.
        assert_eq!(c.file_by_path(&root("/photos/Pics/b.jpg")).unwrap(), Some(FileId::new(2)));
        assert_eq!(c.file_by_path(&root("/photos/pics/b.jpg")).unwrap(), Some(FileId::new(3)));
        assert_eq!(c.files_under(&root("/photos/Pics")).unwrap().len(), 1);
    }

    fn roots_never_nest(rules: PathRules) {
        let c = Catalog::prepare(Path::new(":memory:"), versions(), rules).unwrap();
        assert!(c.add_root(&p(rules, r"D:\a\b")).unwrap());
        assert!(c.add_root(&p(rules, r"D:\a\bc")).unwrap());
        // The same folder spelled another way: on Windows another drive letter and case, which the root covers. POSIX
        // names are case-sensitive, so there the spelling is the same path.
        let same_folder = match rules {
            PathRules::Windows => r"d:\A\b\c",
            PathRules::Posix => "/a/b/c",
        };
        assert!(!c.add_root(&p(rules, same_folder)).unwrap(), "inside an existing root");
        assert!(c.add_root(&p(rules, r"D:\a")).unwrap(), "replaces both");
        let paths =
            |c: &Catalog| c.roots().unwrap().into_iter().map(|r| c.rules().display(&r.path)).collect::<Vec<_>>();
        assert_eq!(paths(&c), [spelled(rules, r"D:\a")]);
        let id = c.root_id(&p(rules, r"D:\a")).unwrap().expect("a library folder has an id");
        assert_eq!(c.remove_root(id).unwrap(), Some(0));
        assert!(paths(&c).is_empty());
        assert_eq!(c.remove_root(id).unwrap(), None, "not a library folder any more");
    }

    under_both_rules!(roots_never_nest, roots_never_nest_windows, roots_never_nest_posix);

    fn cancelled_until_cleared(rules: PathRules) {
        let mut c = Catalog::prepare(Path::new(":memory:"), versions(), rules).unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        c.cancel_when(cancel.clone()).unwrap();
        let run = || raw::slow_count(&c).map_err(Error::from);
        assert_eq!(run().unwrap(), 100_000);
        // Set between statements (SQLite's own interrupt would be lost there): every statement fails.
        cancel.store(true, Ordering::Relaxed);
        assert!(matches!(run().unwrap_err(), Error::Interrupted));
        assert!(matches!(run().unwrap_err(), Error::Interrupted));
        cancel.store(false, Ordering::Relaxed);
        assert_eq!(run().unwrap(), 100_000);
    }

    under_both_rules!(cancelled_until_cleared, cancelled_until_cleared_windows, cancelled_until_cleared_posix);

    /// Whether the catalog at `conn` is this build's schema at its version: the check [`Catalog::prepare`] makes.
    fn is_current(conn: &Connection) -> Result<bool> {
        Ok(mismatch(conn, current_version()?, current_expected()?)?.is_none())
    }

    /// Why the preparation moved the old catalog aside (`None` when it kept it).
    fn rebuilt_why(c: &Catalog) -> Option<String> {
        c.rebuilt().map(|r| r.why.clone())
    }

    /// A catalog prepared at `path`, then changed by `sql`, which is run on it as the file is (closed again).
    fn edited(rules: PathRules, path: &Path, sql: &str) {
        drop(Catalog::prepare(path, versions(), rules).unwrap());
        raw::on_disk(path).execute_batch(sql).unwrap();
    }

    fn preparing_a_catalog_again_rebuilds_nothing(rules: PathRules) {
        let dir = test_folder(SCRATCH, &format!("prepare-twice-{}", rules_name(rules))).unwrap();
        let path = dir.join("catalog.sqlite");
        drop(Catalog::prepare(&path, versions(), rules).unwrap());
        let again = Catalog::prepare(&path, versions(), rules).unwrap();
        assert!(again.rebuilt().is_none());
        drop(again);
        assert!(!dir.join("catalog.sqlite.replaced").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    under_both_rules!(
        preparing_a_catalog_again_rebuilds_nothing,
        preparing_a_catalog_again_rebuilds_nothing_windows,
        preparing_a_catalog_again_rebuilds_nothing_posix
    );

    fn a_changed_definition_with_the_same_names_is_moved_aside(rules: PathRules) {
        // The same object names: an added column, an index on other columns, an object this build doesn't make.
        for (name, sql, named) in [
            ("edited-column", "ALTER TABLE files ADD COLUMN extra INTEGER;", "files"),
            ("edited-index", "DROP INDEX files_changed; CREATE INDEX files_changed ON files(name);", "files_changed"),
            ("edited-table", "CREATE TABLE extra(x INTEGER);", "extra"),
        ] {
            let dir = test_folder(SCRATCH, &format!("{name}-{}", rules_name(rules))).unwrap();
            let path = dir.join("catalog.sqlite");
            edited(rules, &path, sql);
            let c = Catalog::prepare(&path, versions(), rules).unwrap();
            let why = rebuilt_why(&c).unwrap_or_else(|| panic!("{name}: not moved aside"));
            assert!(why.contains(named), "{name}: {why}");
            assert!(is_current(&raw::on_disk(&path)).unwrap());
            drop(c);
            std::fs::remove_dir_all(&dir).unwrap();
        }
    }

    under_both_rules!(
        a_changed_definition_with_the_same_names_is_moved_aside,
        a_changed_definition_with_the_same_names_is_moved_aside_windows,
        a_changed_definition_with_the_same_names_is_moved_aside_posix
    );

    fn a_comment_in_a_stored_definition_is_not_a_schema_change(rules: PathRules) {
        let dir = test_folder(SCRATCH, &format!("comment-edit-{}", rules_name(rules))).unwrap();
        let path = dir.join("catalog.sqlite");
        // A catalog whose `files` table was written with a comment in it: the same schema.
        let commented = BASELINE.replacen("CREATE TABLE files(", "CREATE TABLE files( -- an added comment\n", 1);
        assert_ne!(commented, BASELINE);
        let conn = Connection::open(&path).unwrap();
        bring_to(&conn, &commented, MIGRATIONS, rules, current_version().unwrap()).unwrap();
        drop(conn);
        let c = Catalog::prepare(&path, versions(), rules).unwrap();
        assert!(c.rebuilt().is_none(), "a comment moved the catalog aside");
        drop(c);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    under_both_rules!(
        a_comment_in_a_stored_definition_is_not_a_schema_change,
        a_comment_in_a_stored_definition_is_not_a_schema_change_windows,
        a_comment_in_a_stored_definition_is_not_a_schema_change_posix
    );

    fn a_catalog_at_version_one_is_migrated_forward_and_keeps_its_rows(rules: PathRules) {
        let dir = test_folder(SCRATCH, &format!("migrate-v1-{}", rules_name(rules))).unwrap();
        let path = dir.join("catalog.sqlite");
        // A catalog as the build before the first migration made it: the baseline, with a library folder.
        {
            let conn = Connection::open(&path).unwrap();
            bring_to(&conn, BASELINE, MIGRATIONS, rules, crate::schema::BASELINE_VERSION).unwrap();
            let root = rules.join(&p(rules, r"D:\kept"));
            conn.execute("INSERT INTO roots(id, path, added) VALUES (7, ?1, 1)", [root]).unwrap();
        }
        let c = Catalog::prepare(&path, versions(), rules).unwrap();
        assert!(c.rebuilt().is_none(), "migrated, not rebuilt");
        assert!(!dir.join("catalog.sqlite.replaced").exists());
        assert_eq!(pg_schema::user_version(&c.conn).unwrap(), current_version().unwrap());
        assert!(is_current(&c.conn).unwrap());
        let roots = c.roots().unwrap();
        assert_eq!(roots.len(), 1);
        assert_eq!((roots[0].id, &roots[0].path), (RootId::new(7), &p(rules, r"D:\kept")));
        drop(c);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A file whose extension this build doesn't hold (a build that held more recorded it) is marked missing when the
    /// catalog is prepared, so the grid lists the rest, and every count leaves it out.
    fn a_file_of_an_extension_not_held_is_missing_after_prepare(rules: PathRules) {
        use crate::browse::read::{facets, folder_tree, list_grid};
        use pg_model::query::{Query, Sort};

        let dir = test_folder(SCRATCH, &format!("unheld-ext-{}", rules_name(rules))).unwrap();
        let path = dir.join("catalog.sqlite");
        {
            let mut c = Catalog::prepare(&path, versions(), rules).unwrap();
            c.add_root(&p(rules, r"D:\x")).unwrap();
            let scan = c.begin_scan(&p(rules, r"D:\x")).unwrap();
            for (n, name) in ["a.jpg", "b.jpg"].into_iter().enumerate() {
                let tx = c.tx().unwrap();
                let mut writer = tx.scan_writer(scan, &p(rules, r"D:\x")).unwrap();
                let folder = writer.folder(&p(rules, r"D:\x")).unwrap();
                let file = ScannedFile {
                    identity: FileIdentity::new(1, 10 + u64::try_from(n).unwrap()),
                    name: &Name::from(name),
                    ext: Ext::of_name(name).unwrap(),
                    size: 10,
                    mtime: 100,
                    created: None,
                };
                writer.upsert_file(&folder, &file).unwrap();
                drop(writer);
                tx.commit().unwrap();
            }
            c.conn.execute("UPDATE files SET ext = 'xyz' WHERE name = ?1", [b"b.jpg".to_vec()]).unwrap();
        }
        let c = Catalog::prepare(&path, versions(), rules).unwrap();
        let missing: i64 = c.conn.query_row("SELECT missing FROM files WHERE ext = 'xyz'", [], |r| r.get(0)).unwrap();
        assert_eq!(missing, 1);
        let all = pg_model::rules::Merged::default();
        assert_eq!(list_grid(&c, &all, true, &Query::default(), Sort::default()).unwrap().len(), 1);
        let status = c.status().unwrap();
        assert_eq!((status.files, status.missing), (1, 1));
        assert_eq!(folder_tree(&c).unwrap().total(), 1);
        let counts = facets(&c, &all, true, &Query::default()).unwrap();
        assert_eq!(counts.total, 1);
        assert!(counts.exts.iter().all(|(ext, _)| ext != "xyz"), "{:?}", counts.exts);
        drop(c);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    under_both_rules!(
        a_file_of_an_extension_not_held_is_missing_after_prepare,
        a_file_of_an_extension_not_held_is_missing_after_prepare_windows,
        a_file_of_an_extension_not_held_is_missing_after_prepare_posix
    );

    under_both_rules!(
        a_catalog_at_version_one_is_migrated_forward_and_keeps_its_rows,
        a_catalog_at_version_one_is_migrated_forward_and_keeps_its_rows_windows,
        a_catalog_at_version_one_is_migrated_forward_and_keeps_its_rows_posix
    );

    fn a_database_that_is_not_a_catalog_is_moved_aside_not_built_on(rules: PathRules) {
        let dir = test_folder(SCRATCH, &format!("foreign-database-{}", rules_name(rules))).unwrap();
        let path = dir.join("catalog.sqlite");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch("CREATE TABLE notes(x INTEGER); INSERT INTO notes VALUES (5);").unwrap();
        }
        let c = Catalog::prepare(&path, versions(), rules).unwrap();
        let why = rebuilt_why(&c).expect("moved aside");
        assert!(why.contains("notes"), "{why}");
        let kept = raw::on_disk(&dir.join("catalog.sqlite.replaced"));
        assert_eq!(kept.query_row("SELECT x FROM notes", [], |r| r.get::<_, i64>(0)).unwrap(), 5);
        drop(kept);
        drop(c);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    under_both_rules!(
        a_database_that_is_not_a_catalog_is_moved_aside_not_built_on,
        a_database_that_is_not_a_catalog_is_moved_aside_not_built_on_windows,
        a_database_that_is_not_a_catalog_is_moved_aside_not_built_on_posix
    );

    fn a_file_that_is_not_a_database_is_moved_aside_unread(rules: PathRules) {
        let dir = test_folder(SCRATCH, &format!("not-a-database-{}", rules_name(rules))).unwrap();
        let path = dir.join("catalog.sqlite");
        std::fs::write(&path, vec![b'x'; 8192]).unwrap();
        let c = Catalog::prepare(&path, versions(), rules).unwrap();
        assert_eq!(rebuilt_why(&c).as_deref(), Some("not a database"));
        assert_eq!(std::fs::read(dir.join("catalog.sqlite.replaced")).unwrap(), vec![b'x'; 8192]);
        drop(c);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    under_both_rules!(
        a_file_that_is_not_a_database_is_moved_aside_unread,
        a_file_that_is_not_a_database_is_moved_aside_unread_windows,
        a_file_that_is_not_a_database_is_moved_aside_unread_posix
    );

    fn the_same_schema_under_another_application_id_is_moved_aside(rules: PathRules) {
        let dir = test_folder(SCRATCH, &format!("application-id-{}", rules_name(rules))).unwrap();
        let path = dir.join("catalog.sqlite");
        drop(Catalog::prepare(&path, versions(), rules).unwrap());
        raw::on_disk(&path).pragma_update(None, "application_id", 7).unwrap();
        let c = Catalog::prepare(&path, versions(), rules).unwrap();
        assert_eq!(rebuilt_why(&c).as_deref(), Some("application id 7, not a catalog's"));
        drop(c);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    under_both_rules!(
        the_same_schema_under_another_application_id_is_moved_aside,
        the_same_schema_under_another_application_id_is_moved_aside_windows,
        the_same_schema_under_another_application_id_is_moved_aside_posix
    );

    fn open_refuses_a_catalog_changed_after_it_was_prepared(rules: PathRules) {
        let dir = test_folder(SCRATCH, &format!("changed-after-prepare-{}", rules_name(rules))).unwrap();
        let path = dir.join("catalog.sqlite");
        drop(Catalog::prepare(&path, versions(), rules).unwrap());
        raw::on_disk(&path).execute_batch("CREATE TABLE extra(x INTEGER);").unwrap();
        let e = Catalog::open(&path, rules).err().expect("refused");
        assert!(e.to_string().contains("extra"), "{e}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    under_both_rules!(
        open_refuses_a_catalog_changed_after_it_was_prepared,
        open_refuses_a_catalog_changed_after_it_was_prepared_windows,
        open_refuses_a_catalog_changed_after_it_was_prepared_posix
    );

    fn two_preparations_of_one_catalog_move_it_aside_once(rules: PathRules) {
        let dir = test_folder(SCRATCH, &format!("concurrent-prepare-{}", rules_name(rules))).unwrap();
        let path = dir.join("catalog.sqlite");
        std::fs::write(&path, vec![b'x'; 8192]).unwrap();
        let threads: Vec<_> = (0..2)
            .map(|_| {
                let path = path.clone();
                std::thread::spawn(move || Catalog::prepare(&path, versions(), rules).map(|c| c.rebuilt().is_some()))
            })
            .collect();
        let moved: Vec<bool> = threads.into_iter().map(|t| t.join().unwrap().unwrap()).collect();
        assert_eq!(moved.iter().filter(|m| **m).count(), 1, "{moved:?}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    under_both_rules!(
        two_preparations_of_one_catalog_move_it_aside_once,
        two_preparations_of_one_catalog_move_it_aside_once_windows,
        two_preparations_of_one_catalog_move_it_aside_once_posix
    );

    fn a_memory_catalog_takes_no_lock_file(rules: PathRules) {
        let c = Catalog::prepare(Path::new(":memory:"), versions(), rules).unwrap();
        assert!(c.rebuilt().is_none());
        assert!(!Path::new(":memory:.lock").exists());
    }

    under_both_rules!(
        a_memory_catalog_takes_no_lock_file,
        a_memory_catalog_takes_no_lock_file_windows,
        a_memory_catalog_takes_no_lock_file_posix
    );

    /// A catalog with one file in it, under `rules`.
    fn one_file(rules: PathRules) -> (Catalog, i64) {
        let c = Catalog::prepare(Path::new(":memory:"), versions(), rules).unwrap();
        raw::add_scan(&c, 1, &spelled(rules, "D:"));
        raw::add_folder(&c, 1, &spelled(rules, r"D:\x"));
        fixture::insert(&c, &[fixture::File::new(FileId::new(1), FolderId::new(1), "a.jpg", 10, 100)]).unwrap();
        (c, 1)
    }

    /// A turn the file can't hold, written by the writer: the catalog keeps it.
    fn turn_kept(c: &Catalog, id: FileId, turns: i64) {
        let job =
            writes::Job { file_id: id, kind: pg_model::write::WriteKind::Turns, value: turns, temp: None, attempts: 0 };
        writes::done(c, &job, writes::WriteEffect::KeptInCatalog(Turns::new(u8::try_from(turns).unwrap()))).unwrap();
    }

    /// Whether the import would analyze file `id` now.
    fn due(c: &Catalog, id: FileId) -> bool {
        c.pending_import(false).unwrap().items.iter().any(|i| i.id == id)
    }

    /// Whether file `id` is due for the import, and its change number (`files.seq`).
    fn state(c: &Catalog, id: FileId) -> (bool, i64) {
        (due(c, id), raw::file_seq(c, id))
    }

    /// The scanner finding file 7 on volume 1 in D:\x like this; whether that changed the catalog.
    fn scanned(c: &mut Catalog, scan: i64, name: &str, size: u64, mtime: i64) -> bool {
        let rules = c.rules();
        let tx = c.tx().unwrap();
        let mut writer = tx.scan_writer(scan, &p(rules, r"D:\x")).unwrap();
        let folder = writer.folder(&p(rules, r"D:\x")).unwrap();
        let ext = Ext::of_name(name).unwrap();
        let file =
            ScannedFile { identity: FileIdentity::new(1, 7), name: &Name::from(name), ext, size, mtime, created: None };
        let changed = writer.upsert_file(&folder, &file).unwrap() == Recorded::Changed;
        drop(writer);
        tx.commit().unwrap();
        changed
    }

    fn a_scan_tells_a_file_found_as_it_was_from_one_that_changed(rules: PathRules) {
        let mut c = Catalog::prepare(Path::new(":memory:"), versions(), rules).unwrap();
        c.add_root(&p(rules, r"D:\x")).unwrap();
        let key = |name: &str| {
            let folder = rules.merge(&p(rules, r"D:\x"));
            rules.order_key(&rules.merged_key(&folder), &Name::from(name))
        };
        let scans: Vec<i64> = (0..9).map(|_| c.begin_scan(&p(rules, r"D:\x")).unwrap()).collect();
        assert!(scanned(&mut c, scans[0], "a.jpg", 10, 100), "new");
        // As it was: only marked seen by the scan.
        assert!(!scanned(&mut c, scans[1], "a.jpg", 10, 100));
        assert_eq!(raw::file_row(&c), (scans[1], "a.jpg".into(), 10, 100, 0, key("a.jpg")));
        for (scan, name, size, mtime) in [
            (scans[2], "A.jpg", 10, 100),
            (scans[3], "b.jpg", 10, 100),
            (scans[4], "b.jpg", 11, 100),
            (scans[5], "b.jpg", 11, 101),
        ] {
            assert!(scanned(&mut c, scan, name, size, mtime), "{name} {size} {mtime}");
            assert_eq!(raw::file_row(&c), (scan, name.into(), i64::try_from(size).unwrap(), mtime, 0, key(name)));
        }
        // Back from missing.
        raw::set_missing(&c, FileId::new(1));
        assert!(scanned(&mut c, scans[6], "b.jpg", 11, 101));
        assert_eq!(raw::file_row(&c).4, 0);
        // An order key made by an earlier rule is made again.
        raw::set_order_key(&c, FileId::new(1), &[0]);
        assert!(scanned(&mut c, scans[7], "b.jpg", 11, 101));
        assert_eq!(raw::file_row(&c).5, key("b.jpg"));
        assert!(!scanned(&mut c, scans[8], "b.jpg", 11, 101));
    }

    under_both_rules!(
        a_scan_tells_a_file_found_as_it_was_from_one_that_changed,
        a_scan_tells_a_file_found_as_it_was_from_one_that_changed_windows,
        a_scan_tells_a_file_found_as_it_was_from_one_that_changed_posix
    );

    /// The scanner finding a file in D:\x with this identity and creation time (a.jpg, 10 bytes, mtime 100).
    fn found_as(c: &mut Catalog, scan: i64, identity: FileIdentity, created: Option<i64>) -> Recorded {
        let rules = c.rules();
        let tx = c.tx().unwrap();
        let mut writer = tx.scan_writer(scan, &p(rules, r"D:\x")).unwrap();
        let folder = writer.folder(&p(rules, r"D:\x")).unwrap();
        let file = ScannedFile {
            identity,
            name: &Name::from("a.jpg"),
            ext: Ext::of_name("a.jpg").unwrap(),
            size: 10,
            mtime: 100,
            created,
        };
        let recorded = writer.upsert_file(&folder, &file).unwrap();
        drop(writer);
        tx.commit().unwrap();
        recorded
    }

    fn a_volume_and_file_id_above_i64_max_are_kept_whole_and_found_again(rules: PathRules) {
        let mut c = Catalog::prepare(Path::new(":memory:"), versions(), rules).unwrap();
        let scan = c.begin_scan(&p(rules, r"D:\x")).unwrap();
        let (volume, file) = (u64::MAX - 1, u64::MAX);
        assert_eq!(found_as(&mut c, scan, FileIdentity::new(volume, file), None), Recorded::Changed);
        let stored: (i64, i64) =
            c.conn.query_row("SELECT volume, file_ref FROM files", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!((stored.0.cast_unsigned(), stored.1.cast_unsigned()), (volume, file));
        // The same identity read back from the catalog is the same file: no new row.
        assert_eq!(found_as(&mut c, scan, FileIdentity::new(volume, file), None), Recorded::Same);
    }

    under_both_rules!(
        a_volume_and_file_id_above_i64_max_are_kept_whole_and_found_again,
        a_volume_and_file_id_above_i64_max_are_kept_whole_and_found_again_windows,
        a_volume_and_file_id_above_i64_max_are_kept_whole_and_found_again_posix
    );

    fn a_creation_time_fills_in_once_and_a_different_one_is_a_different_file(rules: PathRules) {
        let mut c = Catalog::prepare(Path::new(":memory:"), versions(), rules).unwrap();
        let scan = c.begin_scan(&p(rules, r"D:\x")).unwrap();
        let identity = FileIdentity::new(1, 7);
        // Unknown on the first scan (a file system that doesn't give it), then known: it is filled in.
        assert_eq!(found_as(&mut c, scan, identity, None), Recorded::Changed);
        assert_eq!(found_as(&mut c, scan, identity, Some(5)), Recorded::Same);
        let created: Option<i64> =
            c.conn.query_row("SELECT created FROM files WHERE volume = 1", [], |r| r.get(0)).unwrap();
        assert_eq!(created, Some(5));
        // A different creation time is a different file: the row is displaced, and a new row takes the identity.
        assert_eq!(found_as(&mut c, scan, identity, Some(6)), Recorded::Changed);
        let rows: i64 = c.conn.query_row("SELECT count(*) FROM files", [], |r| r.get(0)).unwrap();
        assert_eq!(rows, 2);
        let displaced: bool = c
            .conn
            .query_row(
                "SELECT volume IS NULL AND unpaired = 1 AND created = 5 FROM files WHERE volume IS NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(displaced);
    }

    under_both_rules!(
        a_creation_time_fills_in_once_and_a_different_one_is_a_different_file,
        a_creation_time_fills_in_once_and_a_different_one_is_a_different_file_windows,
        a_creation_time_fills_in_once_and_a_different_one_is_a_different_file_posix
    );

    fn what_was_read_from_a_file_is_kept_only_while_the_file_is_as_listed(rules: PathRules) {
        let mut c = Catalog::prepare(Path::new(":memory:"), versions(), rules).unwrap();
        c.add_root(&p(rules, r"D:\x")).unwrap();
        let scan = c.begin_scan(&p(rules, r"D:\x")).unwrap();
        scanned(&mut c, scan, "a.jpg", 10, 100);
        let stored = |c: &mut Catalog, item: &WorkItem| {
            let tx = c.tx().unwrap();
            let stored = tx.analysis_writer(versions().import).unwrap().store_failure(item, "a stand-in").unwrap();
            tx.commit().unwrap();
            stored
        };
        let item = next_work(&c);
        // A write of the app's into it in progress: it may have been read part-written.
        writes::queue_rotation(&c, item.id, Turns::new(3)).unwrap();
        let job = writes::next_ready(&c, 0).unwrap().unwrap();
        let temp = rules.parse(&spelled(rules, r"D:\x\a.jpg.tmp")).unwrap();
        writes::record_temp(&c, &job, &temp, 3).unwrap();
        assert!(!stored(&mut c, &item));
        assert!(due(&c, item.id), "still due");
        writes::done(&c, &job, writes::WriteEffect::Nothing).unwrap();
        // Changed since it was listed.
        scanned(&mut c, scan, "a.jpg", 11, 101);
        assert!(!stored(&mut c, &item));
        // Turned in the catalog since.
        turn_kept(&c, item.id, 1);
        let listed = next_work(&c);
        assert_eq!(listed.rotation, Turns::new(1));
        let unturned = WorkItem { rotation: Turns::new(0), ..listed };
        assert!(!stored(&mut c, &unturned));
        // As listed.
        assert!(stored(&mut c, &listed));
        assert!(!due(&c, item.id));
    }

    under_both_rules!(
        what_was_read_from_a_file_is_kept_only_while_the_file_is_as_listed,
        what_was_read_from_a_file_is_kept_only_while_the_file_is_as_listed_windows,
        what_was_read_from_a_file_is_kept_only_while_the_file_is_as_listed_posix
    );

    fn a_size_change_alone_moves_the_file_in_the_thumbnail_source(rules: PathRules) {
        // The scanner decides a file changed by its size and mtime, so a size change with the same mtime must reach
        // the thumbnail pass (through files.seq), or a thumbnail made from the old file would be kept.
        use fixture::{File, insert};
        let c = Catalog::prepare(Path::new(":memory:"), versions(), rules).unwrap();
        raw::add_scan(&c, 1, &spelled(rules, r"D:\x"));
        raw::add_folder(&c, 1, &spelled(rules, r"D:\x"));
        insert(&c, &[File::new(FileId::new(1), FolderId::new(1), "a.jpg", 10, 100)]).unwrap();
        raw::insert_analysis(&c, &raw::AnalysisRow::image(1, 10, 100));
        let before = raw::thumb_source_seq(&c, FileId::new(1));
        raw::set_size(&c, FileId::new(1), 11);
        assert!(raw::thumb_source_seq(&c, FileId::new(1)) > before);
    }

    under_both_rules!(
        a_size_change_alone_moves_the_file_in_the_thumbnail_source,
        a_size_change_alone_moves_the_file_in_the_thumbnail_source_windows,
        a_size_change_alone_moves_the_file_in_the_thumbnail_source_posix
    );

    fn thumb_sources_is_the_analysed_files_with_their_folders(rules: PathRules) {
        use fixture::{File, insert};
        let c = Catalog::prepare(Path::new(":memory:"), versions(), rules).unwrap();
        raw::add_scan(&c, 1, &spelled(rules, r"D:\x"));
        raw::add_folder(&c, 1, &spelled(rules, r"D:\x"));
        raw::add_folder(&c, 2, &spelled(rules, r"D:\x\sub"));
        insert(
            &c,
            &[
                File::new(FileId::new(1), FolderId::new(1), "a.jpg", 10, 100),
                File::new(FileId::new(2), FolderId::new(2), "b.jpg", 10, 100),
                File::new(FileId::new(3), FolderId::new(1), "c.jpg", 10, 100),
            ],
        )
        .unwrap();
        // File 1 analyzed, file 2 failed (still a source: the pass leaves it out), file 3 not analyzed (not a source).
        raw::insert_analysis(&c, &raw::AnalysisRow::image(1, 10, 100));
        raw::insert_analysis(
            &c,
            &raw::AnalysisRow { error: Some("unreadable"), ..raw::AnalysisRow::image(2, 10, 100) },
        );
        assert_eq!(raw::thumb_sources(&c), [(1, spelled(rules, r"D:\x")), (2, spelled(rules, r"D:\x\sub"))]);
        // The clock is the catalog's change number, as the pass reads it.
        assert_eq!(raw::thumb_clock(&c), raw::meta_seq(&c));
        // The pass's shape over it, with a TEMP stub of the thumbnail cache's entries: a change number's range still
        // reads files_seq, and a file with no thumbnail yet is the one listed.
        raw::stub_thumb_entries(&c);
        let sql = "SELECT f.id FROM thumb_sources f LEFT JOIN entries t ON t.file_id = f.id AND t.height = ?1
                   WHERE f.seq > ?2 AND f.missing = 0 AND f.error IS NULL AND f.analyzed_mtime = f.mtime
                     AND (t.file_id IS NULL OR t.mtime != f.mtime OR t.rotation != f.rotation)
                   ORDER BY f.volume, f.file_ref & 0xFFFFFFFFFFFF";
        let plan = raw::query_plan(&c, sql, [100, -1]);
        assert!(plan.contains("files_seq"), "{plan:?}");
        let listed = raw::rows_of(&c, sql, [100, -1], |r| r.get::<_, i64>(0));
        assert_eq!(listed, [1]);
    }

    under_both_rules!(
        thumb_sources_is_the_analysed_files_with_their_folders,
        thumb_sources_is_the_analysed_files_with_their_folders_windows,
        thumb_sources_is_the_analysed_files_with_their_folders_posix
    );

    /// The facts of a JPEG with no rating and stand-in hashes, for the analyses the tests store.
    fn a_jpeg() -> Media {
        let pdq = pg_model::pdq::Pdq { hash: pg_model::pdq::Hash([0; 32]), quality: 0 };
        let pixels = PixelHashes { pixel_hash: [0; 16], pdq };
        Media::Image(ImageFacts { format: Format::Jpeg, file_rating: None, pixels })
    }

    /// An analysis as the import holds it: an image's dihedral hashes are stand-ins (`a_jpeg`'s), a video's are none.
    fn stored_analysis(record: &Analysis) -> Analyzed {
        let pdq_dihedral = record.image().map(|_| [pg_model::pdq::Hash([0; 32]); 8]);
        Analyzed { record: record.clone(), pdq_dihedral }
    }

    /// The import storing what it made of `item` (a failure: it counts as made, and needs no Analysis).
    fn analyzed(c: &mut Catalog, item: &WorkItem) {
        let tx = c.tx().unwrap();
        tx.analysis_writer(versions().import).unwrap().store_failure(item, "a stand-in for an analysis").unwrap();
        tx.commit().unwrap();
    }

    fn next_work(c: &Catalog) -> WorkItem {
        let mut work = c.pending_import(false).unwrap();
        assert_eq!(work.len(), 1);
        work.items.remove(0)
    }

    fn borders_are_looked_for_after_the_import_and_again_after_a_new_detector(rules: PathRules) {
        use crate::browse;
        use pg_model::detection::{Edge, Kind as SideKind, Reason, Side};
        use pg_model::query::{BorderFacet, Query, Sort};

        let mut c = Catalog::prepare(Path::new(":memory:"), versions(), rules).unwrap();
        c.add_root(&p(rules, r"D:\x")).unwrap();
        let scan = c.begin_scan(&p(rules, r"D:\x")).unwrap();
        scanned(&mut c, scan, "a.jpg", 10, 100);
        let item = next_work(&c);
        let solid = |edge| Side {
            pass: 0,
            edge,
            kind: SideKind::Solid,
            depth: 41,
            color: Some([0, 0, 0]),
            straight: 1.0,
            dev: Some(0.0),
            drift: Some(0.5),
            rejected: None,
        };
        let sky = Side {
            pass: 0,
            edge: Edge::Left,
            kind: SideKind::Blur,
            depth: 30,
            color: None,
            straight: 0.75,
            dev: None,
            drift: None,
            rejected: Some(Reason::UnpairedBlur),
        };
        let found = Detection {
            width: 400,
            height: 300,
            content: [0, 41, 400, 259],
            sides: [solid(Edge::Top), solid(Edge::Bottom), sky].into_iter().collect(),
        };
        let analysis = Analysis {
            version: IMPORT_VERSION,
            file_hash: [1; 32],
            orientation: Orientation::UPRIGHT,
            width: 400,
            height: 300,
            thumbhash: ThumbHash::default(),
            frames: 1,
            taken: None,
            media: a_jpeg(),
        };
        let tx = c.tx().unwrap();
        tx.analysis_writer(versions().import).unwrap().store_analysis(&item, &stored_analysis(&analysis)).unwrap();
        tx.commit().unwrap();
        // The import leaves them to the borders pass.
        assert_eq!(raw::cut_sides(&c), [None; 4]);
        let due = c.pending_borders().unwrap();
        assert_eq!(due.items.iter().map(|d| d.id).collect::<Vec<_>>(), [item.id]);
        let tx = c.tx().unwrap();
        tx.borders_writer().unwrap().store_borders(&due.items[0], Some(&found)).unwrap();
        tx.commit().unwrap();
        assert_eq!(raw::cut_sides(&c), [Some(0), Some(41), Some(0), Some(41)]);
        assert_eq!(browse::read::border_sides(&c, item.id).unwrap(), found.sides);
        assert!(c.pending_borders().unwrap().is_empty());

        // The grid's entry, the filters and the sort.
        let list = |c: &Catalog, q: Query| -> Vec<pg_model::grid::Entry> {
            browse::read::list_grid(c, &pg_model::rules::Merged::default(), true, &q, Sort::default()).unwrap()
        };
        let entry = list(&c, Query::default()).remove(0);
        assert_eq!(entry.borders, pg_model::grid::Borders::Found);
        let [x, y, w, h] = entry.content().unwrap();
        assert!(x == 0.0 && w == 1.0 && (y - 41.0 / 300.0).abs() < 1e-9 && (h - 218.0 / 300.0).abs() < 1e-9);
        let only = |b: Vec<BorderFacet>| Query { borders: b, ..Query::default() };
        assert_eq!(list(&c, only(vec![BorderFacet::Black])).len(), 1);
        assert!(list(&c, only(vec![BorderFacet::Blur, BorderFacet::None])).is_empty());
        let counts =
            browse::read::facets(&c, &pg_model::rules::Merged::default(), true, &Query::default()).unwrap().borders;
        assert_eq!(counts.iter().find(|(b, _)| *b == BorderFacet::Black).unwrap().1, 1);
        assert_eq!(counts.iter().find(|(b, _)| *b == BorderFacet::Blur).unwrap().1, 0);

        // Marked as not a border: no border to the filters, the mark to the review.
        c.set_border_mark(item.id, true).unwrap();
        assert_eq!(list(&c, Query::default())[0].borders, pg_model::grid::Borders::Marked);
        assert!(list(&c, only(vec![BorderFacet::Black])).is_empty());
        assert_eq!(list(&c, only(vec![BorderFacet::None])).len(), 1);
        assert_eq!(list(&c, Query { border_marked: Some(true), ..Query::default() }).len(), 1);

        // A new detector: due again, without a new import; the mark stays.
        raw::older_borders_version(&c);
        raw::start_borders_version(&c);
        assert!(c.pending_import(false).unwrap().is_empty());
        let due = c.pending_borders().unwrap();
        assert_eq!(due.len(), 1);
        let tx = c.tx().unwrap();
        tx.borders_writer().unwrap().store_borders(&due.items[0], Some(&Detection::none(400, 300))).unwrap();
        tx.commit().unwrap();
        assert_eq!(raw::cut_sides(&c), [Some(0); 4]);
        assert!(browse::read::border_sides(&c, item.id).unwrap().is_empty());
        assert!(c.pending_borders().unwrap().is_empty());
        assert_eq!(list(&c, Query::default())[0].borders, pg_model::grid::Borders::None);
        assert_eq!(raw::border_marks(&c), 1);

        // Analyzed again (the file changed): the old borders go, the new ones are due.
        let tx = c.tx().unwrap();
        tx.borders_writer().unwrap().store_borders(&due.items[0], Some(&found)).unwrap();
        tx.analysis_writer(versions().import).unwrap().store_analysis(&item, &stored_analysis(&analysis)).unwrap();
        tx.commit().unwrap();
        assert_eq!(raw::cut_sides(&c), [None; 4]);
        assert!(browse::read::border_sides(&c, item.id).unwrap().is_empty());
        assert_eq!(c.pending_borders().unwrap().len(), 1);
    }

    under_both_rules!(
        borders_are_looked_for_after_the_import_and_again_after_a_new_detector,
        borders_are_looked_for_after_the_import_and_again_after_a_new_detector_windows,
        borders_are_looked_for_after_the_import_and_again_after_a_new_detector_posix
    );

    /// The borders the grid shows are a change of the file (`changes_since`): storing them, a changed cut, removed
    /// sides and a removed analysis each stamp it. Its thumbnails' source (`files.seq`) is not touched.
    fn borders_stamp_the_file_as_changed_and_not_its_thumbnails(rules: PathRules) {
        use crate::browse::read::changes_since;
        use pg_model::detection::{Edge, Kind as SideKind, Side};

        let mut c = Catalog::prepare(Path::new(":memory:"), versions(), rules).unwrap();
        c.add_root(&p(rules, r"D:\x")).unwrap();
        let scan = c.begin_scan(&p(rules, r"D:\x")).unwrap();
        scanned(&mut c, scan, "a.jpg", 10, 100);
        let item = next_work(&c);
        let analysis = Analysis {
            version: IMPORT_VERSION,
            file_hash: [1; 32],
            orientation: Orientation::UPRIGHT,
            width: 400,
            height: 300,
            thumbhash: ThumbHash::default(),
            frames: 1,
            taken: None,
            media: a_jpeg(),
        };
        let tx = c.tx().unwrap();
        tx.analysis_writer(versions().import).unwrap().store_analysis(&item, &stored_analysis(&analysis)).unwrap();
        tx.commit().unwrap();
        let seq = |c: &Catalog| -> i64 {
            c.conn.query_row("SELECT seq FROM files WHERE id = ?1", [item.id.get()], |r| r.get(0)).unwrap()
        };
        let thumbs = seq(&c);
        // Each step is a change of the file, after the change number before it.
        let stamped = |c: &Catalog, since: i64| -> i64 {
            let (changed, now) = changes_since(c, since).unwrap();
            assert_eq!(changed, [item.id]);
            assert_eq!(seq(c), thumbs, "the thumbnails' source is not changed");
            now
        };
        let (_, before) = changes_since(&c, 0).unwrap();

        let top = Side {
            pass: 0,
            edge: Edge::Top,
            kind: SideKind::Solid,
            depth: 41,
            color: Some([0, 0, 0]),
            straight: 1.0,
            dev: Some(0.0),
            drift: Some(0.5),
            rejected: None,
        };
        let sides = [top].into_iter().collect();
        let found = Detection { width: 400, height: 300, content: [0, 41, 400, 300], sides };
        let tx = c.tx().unwrap();
        tx.borders_writer().unwrap().store_borders(&item, Some(&found)).unwrap();
        tx.commit().unwrap();
        let now = stamped(&c, before);

        // The same cut written again changes nothing.
        c.conn.execute("UPDATE analysis SET cut_top = cut_top WHERE file_id = ?1", [item.id.get()]).unwrap();
        assert_eq!(changes_since(&c, now).unwrap(), (vec![], now));
        c.conn.execute("UPDATE analysis SET cut_top = 7 WHERE file_id = ?1", [item.id.get()]).unwrap();
        let now = stamped(&c, now);
        c.conn.execute("DELETE FROM border_sides WHERE file_id = ?1", [item.id.get()]).unwrap();
        let now = stamped(&c, now);
        c.conn.execute("DELETE FROM analysis WHERE file_id = ?1", [item.id.get()]).unwrap();
        stamped(&c, now);
    }

    under_both_rules!(
        borders_stamp_the_file_as_changed_and_not_its_thumbnails,
        borders_stamp_the_file_as_changed_and_not_its_thumbnails_windows,
        borders_stamp_the_file_as_changed_and_not_its_thumbnails_posix
    );

    fn border_depth_sorts_by_the_largest_share_of_a_side_cut(rules: PathRules) {
        use crate::browse;
        use pg_model::query::{Query, Sort, SortKey};

        let mut c = Catalog::prepare(Path::new(":memory:"), versions(), rules).unwrap();
        c.add_root(&p(rules, r"D:\x")).unwrap();
        let scan = c.begin_scan(&p(rules, r"D:\x")).unwrap();
        // 400 × 300 stored. b: none; c: 30 of 300 at the top (10%); d: 60 of 400 at the left (15%), but turned a
        // quarter (orientation 6), so 60 of the 400 upright rows; e: 90 at the top (30%), marked not a border.
        let files = [("b.jpg", 1, None), ("c.jpg", 1, Some([0, 30, 0, 0])), ("d.jpg", 6, Some([60, 0, 0, 0]))];
        let files = files.into_iter().chain([("e.jpg", 1, Some([0, 90, 0, 0]))]);
        for (n, (name, orientation, cut)) in files.enumerate() {
            let tx = c.tx().unwrap();
            let mut writer = tx.scan_writer(scan, &p(rules, r"D:\x")).unwrap();
            let folder = writer.folder(&p(rules, r"D:\x")).unwrap();
            let ext = Ext::of_name(name).unwrap();
            let file = ScannedFile {
                identity: FileIdentity::new(1, 10 + u64::try_from(n).unwrap()),
                name: &Name::from(name),
                ext,
                size: 10,
                mtime: 100,
                created: None,
            };
            writer.upsert_file(&folder, &file).unwrap();
            drop(writer);
            tx.commit().unwrap();
            let item = c.pending_import(false).unwrap().items.remove(0);
            let [l, t, r, b] = cut.unwrap_or([0; 4]);
            let mut borders = Detection::none(400, 300);
            borders.content = [l, t, 400 - r, 300 - b];
            let (width, height) = if Orientation::from_tag(orientation).swaps_axes() { (300, 400) } else { (400, 300) };
            let analysis = Analysis {
                version: IMPORT_VERSION,
                file_hash: [u8::try_from(n).unwrap(); 32],
                orientation: Orientation::from_tag(orientation),
                width,
                height,
                thumbhash: ThumbHash::default(),
                frames: 1,
                taken: None,
                media: a_jpeg(),
            };
            let tx = c.tx().unwrap();
            tx.analysis_writer(versions().import).unwrap().store_analysis(&item, &stored_analysis(&analysis)).unwrap();
            tx.borders_writer().unwrap().store_borders(&item, Some(&borders)).unwrap();
            tx.commit().unwrap();
            if name == "e.jpg" {
                c.set_border_mark(item.id, true).unwrap();
            }
        }
        let names = |descending: bool| -> Vec<String> {
            let sort = Sort { key: SortKey::Borders, descending };
            let entries =
                browse::read::list_grid(&c, &pg_model::rules::Merged::default(), true, &Query::default(), sort)
                    .unwrap();
            let name = |id| {
                browse::read::details(&c, id).unwrap().unwrap().path.parts.last().map(|n| n.display().into_owned())
            };
            entries.iter().map(|e| name(e.id).unwrap_or_default()).collect()
        };
        // Without a border (none found, or marked) last, by name, either way.
        assert_eq!(names(false), ["c.jpg", "d.jpg", "b.jpg", "e.jpg"]);
        assert_eq!(names(true), ["d.jpg", "c.jpg", "b.jpg", "e.jpg"]);
    }

    under_both_rules!(
        border_depth_sorts_by_the_largest_share_of_a_side_cut,
        border_depth_sorts_by_the_largest_share_of_a_side_cut_windows,
        border_depth_sorts_by_the_largest_share_of_a_side_cut_posix
    );

    /// Stores an import of `item` with `taken` as its time, as the import stores it.
    fn store_taken(c: &mut Catalog, item: &WorkItem, taken: Option<Taken>) {
        let analysis = Analysis {
            version: IMPORT_VERSION,
            file_hash: [1; 32],
            orientation: Orientation::UPRIGHT,
            width: 400,
            height: 300,
            thumbhash: ThumbHash::default(),
            frames: 1,
            taken,
            media: a_jpeg(),
        };
        let tx = c.tx().unwrap();
        tx.analysis_writer(versions().import).unwrap().store_analysis(item, &stored_analysis(&analysis)).unwrap();
        tx.commit().unwrap();
    }

    /// `analysis.taken` and `analysis.taken_utc` of file 1.
    fn taken_columns(c: &Catalog) -> (Option<i64>, Option<i64>) {
        c.conn
            .query_row("SELECT taken, taken_utc FROM analysis WHERE file_id = 1", [], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
    }

    /// The local wall clock of a UTC instant in this system's zone, as SQLite converts it.
    fn local_of(c: &Catalog, instant: i64) -> i64 {
        c.conn.query_row("SELECT unixepoch(?1, 'unixepoch', 'localtime')", [instant], |r| r.get(0)).unwrap()
    }

    fn a_camera_clock_is_kept_as_it_is(rules: PathRules) {
        let (mut c, _) = one_file(rules);
        let item = next_work(&c);
        store_taken(&mut c, &item, Some(Taken::Wall(1_700_000_000)));
        assert_eq!(taken_columns(&c), (Some(1_700_000_000), None));
    }

    under_both_rules!(
        a_camera_clock_is_kept_as_it_is,
        a_camera_clock_is_kept_as_it_is_windows,
        a_camera_clock_is_kept_as_it_is_posix
    );

    fn a_utc_instant_is_converted_to_the_local_wall_clock_and_kept_as_well(rules: PathRules) {
        let (mut c, _) = one_file(rules);
        let item = next_work(&c);
        let instant = 1_714_557_600;
        store_taken(&mut c, &item, Some(Taken::Utc(instant)));
        assert_eq!(taken_columns(&c), (Some(local_of(&c, instant)), Some(instant)));
    }

    under_both_rules!(
        a_utc_instant_is_converted_to_the_local_wall_clock_and_kept_as_well,
        a_utc_instant_is_converted_to_the_local_wall_clock_and_kept_as_well_windows,
        a_utc_instant_is_converted_to_the_local_wall_clock_and_kept_as_well_posix
    );

    fn a_changed_zone_computes_utc_taken_times_again_from_their_instants(rules: PathRules) {
        let (mut c, _) = one_file(rules);
        let item = next_work(&c);
        let instant = 1_714_557_600;
        store_taken(&mut c, &item, Some(Taken::Utc(instant)));

        // A `taken` left from another zone, and a fingerprint no zone has (a UTC system's own is "0,0,0,0", so a
        // numeric stand-in could match it and hide the change): the next prepare recomputes it.
        c.conn.execute("UPDATE analysis SET taken = 0 WHERE file_id = 1", []).unwrap();
        c.conn.execute("UPDATE meta SET value = 'another-zone' WHERE key = 'taken_zone'", []).unwrap();
        set_taken_zone(&c.conn).unwrap();
        assert_eq!(taken_columns(&c), (Some(local_of(&c, instant)), Some(instant)));
        let current: String = c.conn.query_row(TAKEN_ZONE_QUERY, [], |r| r.get(0)).unwrap();
        let stored: String =
            c.conn.query_row("SELECT value FROM meta WHERE key = 'taken_zone'", [], |r| r.get(0)).unwrap();
        assert_eq!(stored, current);
        assert_eq!(current.split(',').count(), 4);
    }

    under_both_rules!(
        a_changed_zone_computes_utc_taken_times_again_from_their_instants,
        a_changed_zone_computes_utc_taken_times_again_from_their_instants_windows,
        a_changed_zone_computes_utc_taken_times_again_from_their_instants_posix
    );

    fn a_camera_clock_is_not_changed_by_a_changed_zone(rules: PathRules) {
        let (mut c, _) = one_file(rules);
        let item = next_work(&c);
        store_taken(&mut c, &item, Some(Taken::Wall(1_700_000_000)));
        c.conn.execute("UPDATE meta SET value = 'another-zone' WHERE key = 'taken_zone'", []).unwrap();
        set_taken_zone(&c.conn).unwrap();
        assert_eq!(taken_columns(&c), (Some(1_700_000_000), None));
    }

    under_both_rules!(
        a_camera_clock_is_not_changed_by_a_changed_zone,
        a_camera_clock_is_not_changed_by_a_changed_zone_windows,
        a_camera_clock_is_not_changed_by_a_changed_zone_posix
    );

    fn prepare_records_the_zone_its_utc_times_were_computed_with(rules: PathRules) {
        let c = Catalog::prepare(Path::new(":memory:"), versions(), rules).unwrap();
        let current: String = c.conn.query_row(TAKEN_ZONE_QUERY, [], |r| r.get(0)).unwrap();
        let stored: String =
            c.conn.query_row("SELECT value FROM meta WHERE key = 'taken_zone'", [], |r| r.get(0)).unwrap();
        assert_eq!(stored, current);
    }

    under_both_rules!(
        prepare_records_the_zone_its_utc_times_were_computed_with,
        prepare_records_the_zone_its_utc_times_were_computed_with_windows,
        prepare_records_the_zone_its_utc_times_were_computed_with_posix
    );

    fn pending_follows_what_the_analysis_was_made_from(rules: PathRules) {
        let mut c = Catalog::prepare(Path::new(":memory:"), versions(), rules).unwrap();
        let scan = c.begin_scan(&p(rules, r"D:\x")).unwrap();
        // New: pending.
        scanned(&mut c, scan, "a.jpg", 10, 100);
        let first = next_work(&c);
        let id = first.id;
        assert!(due(&c, id));
        // Analyzed as it is: done. A failure counts as done; only --retry redoes it.
        analyzed(&mut c, &first);
        assert!(!due(&c, id));
        assert!(c.pending_import(false).unwrap().is_empty());
        assert_eq!(c.pending_import(true).unwrap().len(), 1);
        // A rescan that changes nothing changes nothing, for the import or the thumbnails.
        let seq = raw::file_seq(&c, id);
        scanned(&mut c, scan, "a.jpg", 10, 100);
        assert_eq!(state(&c, id), (false, seq));
        // Changed while it was being analyzed: what's stored is of the old file.
        let old = c.pending_import(true).unwrap().items.remove(0);
        scanned(&mut c, scan, "a.jpg", 11, 101);
        analyzed(&mut c, &old);
        assert!(due(&c, id));
        let current = next_work(&c);
        analyzed(&mut c, &current);
        assert!(!due(&c, id));
        // Turned in the catalog: analyzed again with the turn, and the thumbnails' source moved on.
        turn_kept(&c, id, 1);
        let (pending, after) = state(&c, id);
        assert!(pending && after > seq);
        let turned = next_work(&c);
        assert_eq!(turned.rotation, Turns::new(1));
        analyzed(&mut c, &turned);
        // Rewritten by the app: a metadata write carries the analysis along; a pixel write leaves it stale.
        let rewritten = |c: &Catalog, before, after, metadata_only| {
            // A write that leaves what the file holds as it was (no turn), so only its size and mtime change.
            let job =
                writes::Job { file_id: id, kind: pg_model::write::WriteKind::Turns, value: 0, temp: None, attempts: 0 };
            let rewrite = writes::Rewrite { before, after, metadata_only };
            writes::done(c, &job, writes::WriteEffect::TurnedBack { rewrite, turns: Turns::NONE }).unwrap();
        };
        rewritten(&c, (11, 101), (12, 102), Some([0; 32]));
        assert!(!due(&c, id));
        rewritten(&c, (12, 102), (13, 103), None);
        assert!(due(&c, id));
        let rewritten = next_work(&c);
        analyzed(&mut c, &rewritten);
        // Renamed to another kind: analyzed again as that.
        scanned(&mut c, scan, "a.mp4", 13, 103);
        let renamed = next_work(&c);
        assert_eq!(renamed.kind, Kind::Video);
        analyzed(&mut c, &renamed);
        assert!(!due(&c, id));
        // Gone, then back: nothing to import, but the thumbnails look at it both times.
        let seq = raw::file_seq(&c, id);
        let next = c.begin_scan(&p(rules, r"D:\x")).unwrap();
        c.mark_missing(&p(rules, r"D:\x"), next, &[]).unwrap();
        let gone = raw::file_seq(&c, id);
        scanned(&mut c, next, "a.mp4", 13, 103);
        assert_eq!(c.pending_import(false).unwrap().len(), 0);
        assert!(seq < gone && gone < raw::file_seq(&c, id));
    }

    under_both_rules!(
        pending_follows_what_the_analysis_was_made_from,
        pending_follows_what_the_analysis_was_made_from_windows,
        pending_follows_what_the_analysis_was_made_from_posix
    );

    fn a_folder_the_scan_could_not_list_keeps_its_files(rules: PathRules) {
        let mut c = Catalog::prepare(Path::new(":memory:"), versions(), rules).unwrap();
        let put = |c: &mut Catalog, scan: i64, folder: &str, name: &str, file_ref: u64| {
            let tx = c.tx().unwrap();
            let mut writer = tx.scan_writer(scan, &p(rules, r"D:\x")).unwrap();
            let folder = writer.folder(&p(rules, folder)).unwrap();
            let ext = Ext::of_name(name).unwrap();
            let identity = FileIdentity::new(1, file_ref);
            writer
                .upsert_file(
                    &folder,
                    &ScannedFile { identity, name: &Name::from(name), ext, size: 1, mtime: 1, created: None },
                )
                .unwrap();
            drop(writer);
            tx.commit().unwrap();
        };
        let first = c.begin_scan(&p(rules, r"D:\x")).unwrap();
        put(&mut c, first, r"D:\x\ok", "a.jpg", 1);
        put(&mut c, first, r"D:\x\locked", "b.jpg", 2);
        put(&mut c, first, r"D:\x\locked\deep", "c.jpg", 3);

        // The next scan lists only D:\x\ok: D:\x\locked couldn't be read, so its files weren't seen, and aren't missing.
        let next = c.begin_scan(&p(rules, r"D:\x")).unwrap();
        put(&mut c, next, r"D:\x\ok", "a.jpg", 1);
        assert_eq!(c.mark_missing(&p(rules, r"D:\x"), next, &[p(rules, r"D:\x\locked")]).unwrap(), 0);
        assert_eq!(raw::missing_count(&c), 0);

        // Once it can be listed again, a scan that doesn't see its files marks them.
        let later = c.begin_scan(&p(rules, r"D:\x")).unwrap();
        put(&mut c, later, r"D:\x\ok", "a.jpg", 1);
        assert_eq!(c.mark_missing(&p(rules, r"D:\x"), later, &[]).unwrap(), 2);
        assert_eq!(raw::missing_count(&c), 2);
    }

    under_both_rules!(
        a_folder_the_scan_could_not_list_keeps_its_files,
        a_folder_the_scan_could_not_list_keeps_its_files_windows,
        a_folder_the_scan_could_not_list_keeps_its_files_posix
    );

    fn files_marked_are_imported_again_once(rules: PathRules) {
        let mut c = Catalog::prepare(Path::new(":memory:"), versions(), rules).unwrap();
        raw::add_scan(&c, 1, &spelled(rules, "D:"));
        raw::add_folder(&c, 1, &spelled(rules, r"D:\x"));
        raw::add_folder(&c, 2, &spelled(rules, r"D:\x\y"));
        raw::add_folder(&c, 3, &spelled(rules, r"D:\xy"));
        raw::add_folder(&c, 4, &spelled(rules, r"D:\x\y\z"));
        let files = [(1, 1, "a.jpg"), (2, 2, "b.jpg"), (3, 3, "c.jpg"), (4, 4, "d.jpg"), (5, 4, "e.jpg")];
        let files =
            files.map(|(id, folder, name)| fixture::File::new(FileId::new(id), FolderId::new(folder), name, 10, 100));
        fixture::insert(&c, &files).unwrap();
        for item in c.pending_import(false).unwrap().items {
            analyzed(&mut c, &item);
        }
        // Analyzed, then gone.
        raw::set_missing(&c, FileId::new(5));
        assert!(c.pending_import(false).unwrap().is_empty());

        // A folder holds the files of its subfolders, not those of a folder named like it; missing ones aren't there.
        let mut under = c.files_under(&p(rules, r"D:\x\y")).unwrap();
        under.sort();
        assert_eq!(under, [FileId::new(2), FileId::new(4)]);
        assert_eq!(c.files_under(&p(rules, r"D:\x")).unwrap().len(), 3);
        assert!(c.files_under(&p(rules, r"D:\q")).unwrap().is_empty());

        // Marked: due, whatever their analysis, the missing one left; the change number doesn't move.
        let seq = raw::file_seq(&c, FileId::new(2));
        assert_eq!(c.mark_pending(&[FileId::new(2), FileId::new(4), FileId::new(5)]).unwrap(), 2);
        assert_eq!(state(&c, FileId::new(2)), (true, seq));
        assert!(due(&c, FileId::new(4)) && !due(&c, FileId::new(5)) && !due(&c, FileId::new(1)));
        let work = c.pending_import(false).unwrap();
        assert_eq!(work.items.iter().map(|i| i.id).collect::<Vec<_>>(), [FileId::new(2), FileId::new(4)]);
        // Analyzed again, as they are: done.
        for item in work.items {
            analyzed(&mut c, &item);
        }
        assert!(c.pending_import(false).unwrap().is_empty());
    }

    under_both_rules!(
        files_marked_are_imported_again_once,
        files_marked_are_imported_again_once_windows,
        files_marked_are_imported_again_once_posix
    );

    fn a_new_import_version_redoes_everything(rules: PathRules) {
        let (mut c, _) = one_file(rules);
        let item = next_work(&c);
        analyzed(&mut c, &item);
        // As an older build left it: its import version, and analyses made by it.
        raw::older_import_version(&c);
        let (pending, seq) = state(&c, item.id);
        assert!(!pending, "current for that build");
        raw::start_import_version(&c);
        let (pending, after) = state(&c, item.id);
        assert!(pending && after > seq);
        // Done once.
        raw::start_import_version(&c);
        assert_eq!(raw::file_seq(&c, item.id), after);
    }

    under_both_rules!(
        a_new_import_version_redoes_everything,
        a_new_import_version_redoes_everything_windows,
        a_new_import_version_redoes_everything_posix
    );

    fn a_round_with_nothing_to_import_reads_one_index(rules: PathRules) {
        let (c, _) = one_file(rules);
        let plan = raw::query_plan(&c, pending_import_sql(false), []);
        assert!(plan.contains("USING INDEX files_to_import") && !plan.contains("TEMP B-TREE"), "{plan}");
        let retry = raw::query_plan(&c, pending_import_sql(true), []);
        assert!(retry.contains("files_to_import") && retry.contains("analysis_failed"), "{retry}");
    }

    under_both_rules!(
        a_round_with_nothing_to_import_reads_one_index,
        a_round_with_nothing_to_import_reads_one_index_windows,
        a_round_with_nothing_to_import_reads_one_index_posix
    );

    fn set_rating(c: &Catalog, id: i64, n: u8) {
        let stars = Rating::new(n).filter(|r| r.get() > 0);
        writes::set_rating(c, FileId::new(id), stars, false).unwrap();
    }

    /// The import's sync of the file's rating, in a transaction of its own.
    fn sync_file(c: &mut Catalog, id: i64, in_file: Option<Rating>) {
        let tx = c.tx().unwrap();
        tx.analysis_writer(versions().import).unwrap().sync_file_rating(FileId::new(id), in_file).unwrap();
        tx.commit().unwrap();
    }

    fn three_way_rating_sync(rules: PathRules) {
        let (mut c, id) = one_file(rules);
        // First import of a file rated elsewhere (ImageRater, Explorer): adopted.
        sync_file(&mut c, id, Rating::new(4));
        assert_eq!(raw::rating(&c, id), Some(4));
        // Rated in the app, not written back (or the format can't hold it): the file still says 4, as known, so
        // re-imports keep the catalog's 2.
        set_rating(&c, id, 2);
        sync_file(&mut c, id, Rating::new(4));
        assert_eq!(raw::rating(&c, id), Some(2));
        // Another program changes the file's rating: that is newer, and adopted.
        sync_file(&mut c, id, Rating::new(5));
        assert_eq!(raw::rating(&c, id), Some(5));
        // Another program clears it (writes 0, or removes it): the catalog follows.
        sync_file(&mut c, id, Rating::new(0));
        assert_eq!(raw::rating(&c, id), None);
        sync_file(&mut c, id, None);
        assert_eq!(raw::rating(&c, id), None);
        // A write of the app's own still queued wins over the file: a JPEG the app rates 3, queued for the file.
        raw::insert_analysis(&c, &raw::AnalysisRow { format: Some("jpeg"), ..raw::AnalysisRow::image(id, 10, 100) });
        writes::set_rating(&c, FileId::new(id), Rating::new(3), true).unwrap();
        sync_file(&mut c, id, Rating::new(1));
        assert_eq!(raw::rating(&c, id), Some(3));
    }

    under_both_rules!(three_way_rating_sync, three_way_rating_sync_windows, three_way_rating_sync_posix);

    /// A file rated before its first import: the rating is queued for the file (its format isn't known yet), and the
    /// import, which finds no rating in the file, keeps the catalog's while the write is queued.
    fn a_rating_made_before_the_import_is_kept_and_queued(rules: PathRules) {
        let (mut c, id) = one_file(rules);
        writes::set_rating(&c, FileId::new(id), Rating::new(4), true).unwrap();
        assert_eq!(raw::queued(&c, id, "rating"), Some(4), "queued with no analysis");
        sync_file(&mut c, id, None);
        assert_eq!(raw::rating(&c, id), Some(4));
        assert_eq!(raw::queued(&c, id, "rating"), Some(4));
    }

    under_both_rules!(
        a_rating_made_before_the_import_is_kept_and_queued,
        a_rating_made_before_the_import_is_kept_and_queued_windows,
        a_rating_made_before_the_import_is_kept_and_queued_posix
    );
}
