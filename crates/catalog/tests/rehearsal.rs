//! A rehearsal of the catalog's upgrade on a copy of a real catalog, and of a folder's removal on that copy. Not run by
//! `cargo test`: it needs a catalog, named by `PG_REHEARSAL_CATALOG`, and the original is never opened for writing (the
//! test copies it first). Run it with:
//!
//!     $env:PG_REHEARSAL_CATALOG = 'D:\...\catalog.sqlite'
//!     cargo test -p pg-catalog --test rehearsal --locked -- --ignored --nocapture
//!
//! It prints the sizes and the times it took, and fails when a row is lost, a foreign key breaks, a sequence falls behind
//! its table's highest id, or the upgrade's copy of the old file isn't kept. A catalog the upgrade can't migrate in place
//! is rebuilt, and the rebuild's first difference and what it carried are printed.

#![expect(clippy::unwrap_used, reason = "a rehearsal: its failure is the test's")]

use std::path::{Path, PathBuf};
use std::time::Instant;

use pg_catalog::{Catalog, Versions};
use rusqlite::{Connection, OpenFlags};

/// The versions the library passes for this build's analyses (`pg_library::versions`).
fn versions() -> Versions {
    Versions {
        import: pg_analysis::analyze::IMPORT_VERSION,
        borders: pg_analysis::borders::BORDERS_VERSION,
        thumbs: pg_render::THUMBS_VERSION,
        pairing: pg_model::pdq::PAIRING_VERSION,
    }
}

/// The row count of every table in the file, read without changing it.
fn counts(path: &Path) -> Vec<(String, i64)> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let names: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_schema WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    names
        .into_iter()
        .map(|name| {
            let n: i64 = conn.query_row(&format!("SELECT count(*) FROM \"{name}\""), [], |r| r.get(0)).unwrap();
            (name, n)
        })
        .collect()
}

/// The file's size in whole mebibytes.
fn mebibytes(path: &Path) -> u64 {
    std::fs::metadata(path).map_or(0, |m| m.len() / (1 << 20))
}

/// The catalog's integrity after the upgrade or the removal: foreign keys, sequences and the auto-vacuum.
fn check(path: &Path, label: &str) {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let broken: Vec<String> = conn
        .prepare("PRAGMA foreign_key_check")
        .unwrap()
        .query_map([], |r| Ok(format!("{}:{}", r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert!(broken.is_empty(), "{label}: foreign keys broken: {broken:?}");
    for table in ["files", "folders", "roots"] {
        let (seq, max): (Option<i64>, Option<i64>) = conn
            .query_row(
                &format!(
                    "SELECT (SELECT seq FROM sqlite_sequence WHERE name = '{table}'), (SELECT max(id) FROM {table})"
                ),
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        let (seq, max) = (seq.unwrap_or(0), max.unwrap_or(0));
        assert!(seq >= max, "{label}: {table}'s sequence {seq} is behind its highest id {max}");
    }
    let auto_vacuum: i64 = conn.query_row("PRAGMA auto_vacuum", [], |r| r.get(0)).unwrap();
    assert_eq!(auto_vacuum, 2, "{label}: incremental auto-vacuum is on");
    let owed: Option<i64> = conn.query_row("SELECT value FROM meta WHERE key = 'vacuum_owed'", [], |r| r.get(0)).ok();
    assert!(owed.unwrap_or(0) == 0, "{label}: the one-time VACUUM is done");
}

/// The old file as the upgrade kept it: a migration keeps `catalog.sqlite.pre-v<N>.bak`, a rebuild `catalog.sqlite.replaced`.
fn kept_copies(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| (n.contains(".pre-v") && n.ends_with(".bak")) || n.ends_with(".replaced"))
        })
        .collect()
}

#[test]
#[ignore = "a rehearsal: PG_REHEARSAL_CATALOG names a real catalog (see the module's doc)"]
fn the_upgrade_and_a_removal_keep_every_row_and_leave_the_catalog_clean() {
    let Some(source) = std::env::var_os("PG_REHEARSAL_CATALOG") else {
        panic!("PG_REHEARSAL_CATALOG names the catalog to rehearse on");
    };
    let source = PathBuf::from(source);
    let dir = pg_testkit::run_folder("rehearsal", "upgrade").unwrap();
    let copy = dir.join("catalog.sqlite");
    std::fs::copy(&source, &copy).unwrap();
    println!("rehearsal copy: {} ({} MiB)", copy.display(), mebibytes(&copy));

    let before = counts(&copy);
    println!("tables before: {}", before.len());
    let source_version: i64 = Connection::open_with_flags(&copy, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .unwrap()
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();

    let started = Instant::now();
    let catalog = Catalog::prepare(&copy, versions(), pg_platform::native::PATH_RULES).expect("the upgrade runs");
    println!("upgrade: {:.2?}, file now {} MiB", started.elapsed(), mebibytes(&copy));
    if let Some(rebuilt) = catalog.rebuilt() {
        println!("REBUILT: first difference: {}", rebuilt.why);
        match &rebuilt.carried {
            pg_model::notice::Carried::Rows { files, folders } => {
                println!("  carried {files} files, {folders} folders")
            }
            pg_model::notice::Carried::Nothing { reason } => println!("  carried NOTHING: {reason}"),
        }
        println!("  old file kept at {}", rebuilt.kept.display());
    }
    drop(catalog);

    check(&copy, "after the upgrade");
    let after = counts(&copy);
    // The user's data must come through exactly; a table the upgrade writes rows into (its own `meta` keys, a migration's
    // notes) may grow, and no table may lose a row. Every change is printed.
    const EXACT: &[&str] = &["files", "folders", "roots", "ratings", "file_writes", "scans"];
    let mut lost = Vec::new();
    for (table, n) in &before {
        match after.iter().find(|(t, _)| t == table) {
            Some((_, m)) if m == n => {}
            Some((_, m)) => {
                println!("{table}: {n} rows before, {m} after");
                if *m < *n || EXACT.contains(&table.as_str()) {
                    lost.push(format!("{table}: {n} rows before, {m} after"));
                }
            }
            None => lost.push(format!("{table}: gone")),
        }
    }
    assert!(lost.is_empty(), "the upgrade lost or changed rows:\n{}", lost.join("\n"));
    let added: Vec<&str> =
        after.iter().map(|(t, _)| t.as_str()).filter(|t| !before.iter().any(|(b, _)| b == t)).collect();
    println!("tables added by the upgrade: {added:?}");

    // A catalog already at this build's schema is not upgraded, so no copy of the old file is made for it.
    let kept = kept_copies(&dir);
    if source_version < 19 {
        assert!(!kept.is_empty(), "the old file is kept beside the catalog");
    } else {
        println!("already at schema {source_version}: nothing upgraded, no copy kept");
    }
    for k in &kept {
        println!("kept: {} ({} MiB)", k.display(), mebibytes(k));
    }

    // A folder's removal on the upgraded copy: the library's first folder goes, for good.
    let catalog = Catalog::prepare(&copy, versions(), pg_platform::native::PATH_RULES).unwrap();
    // The hidden folders no library folder covers (an older build's removals) get their markers, as the app does on start.
    let started = Instant::now();
    let adopted = catalog.adopt_hidden_folders().unwrap();
    println!("adopted {adopted} hidden folders in {:.2?}", started.elapsed());
    let roots = catalog.roots().unwrap();
    println!("library folders: {}", roots.len());
    let Some(root) = roots.first() else {
        println!("no library folder to remove: the removal is not rehearsed");
        return;
    };
    let files_before = counts(&copy).into_iter().find(|(t, _)| t == "files").unwrap().1;
    let started = Instant::now();
    catalog.begin_removal(root.id, &root.path).unwrap();
    catalog.hide_pages(&root.path).unwrap();
    let driven = catalog.drive_removals(&mut |_| true).unwrap();
    let folder = pg_platform::native::PATH_RULES.display(&root.path);
    println!(
        "removal of {folder}: {} files deleted, {} deferred, in {:.2?}",
        driven.files,
        driven.deferred,
        started.elapsed()
    );
    let started = Instant::now();
    catalog.shrink().unwrap();
    println!("shrink: {:.2?}, file now {} MiB", started.elapsed(), mebibytes(&copy));
    drop(catalog);

    check(&copy, "after the removal");
    let files_after = counts(&copy).into_iter().find(|(t, _)| t == "files").unwrap().1;
    assert_eq!(
        usize::try_from(files_before - files_after).unwrap(),
        driven.files,
        "the rows that went are the rows the removal counted"
    );
}
