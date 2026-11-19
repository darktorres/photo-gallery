//! The baked baseline (phase 2): `baseline.sql` is the schema a fresh catalog has at version 20, written out as literal SQL
//! from the migration chain, and the gate that proves the two equal before the chain is deleted (2.3).
//!
//! `write_baseline_from_v20` regenerates the file from the chain. It is run once, by hand; afterwards the file is the
//! source and the generator is not run again.

use std::fmt::Write as _;
use std::path::Path;

use rusqlite::Connection;
use rusqlite::types::Value;

use crate::schema::{Hooks, SCHEMA, bring_to, current_version};

/// The baked baseline: the schema a fresh catalog has at version 20 (see the module docs).
pub(crate) const BAKED: &str = include_str!("baseline.sql");

const HEADER: &str =
    "-- The catalog's schema as the migration chain makes it at version 20: the objects in creation order
-- (tables, indexes, views, triggers), then the rows the chain seeds in `meta`. Plain SQL: the macros were expanded
-- when it was made. `bring_to` adds the `path_rules` row, the application id and the version.\n\n";

#[test]
#[ignore = "writes baseline.sql from the v20 chain: run once, by hand"]
fn write_baseline_from_v20() {
    let conn = Connection::open_in_memory().unwrap();
    bring_to(&conn, &SCHEMA, pg_platform::native::PATH_RULES, current_version().unwrap(), &Hooks::none()).unwrap();
    let mut out = String::from(HEADER);
    for kind in ["table", "index", "view", "trigger"] {
        writeln!(out, "-- {kind}s").unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT sql FROM sqlite_schema WHERE type = ?1 AND sql IS NOT NULL AND name NOT LIKE 'sqlite%' ORDER BY rowid",
            )
            .unwrap();
        for sql in stmt.query_map([kind], |r| r.get::<_, String>(0)).unwrap() {
            writeln!(out, "{};\n", sql.unwrap()).unwrap();
        }
    }
    // The chain must leave every table empty but `meta`: the baseline carries no data.
    let mut tables = conn
        .prepare("SELECT name FROM sqlite_schema WHERE type = 'table' AND name NOT LIKE 'sqlite%' AND name <> 'meta'")
        .unwrap();
    let names: Vec<String> = tables.query_map([], |r| r.get(0)).unwrap().map(Result::unwrap).collect();
    for name in names {
        let rows: i64 = conn.query_row(&format!("SELECT count(*) FROM \"{name}\""), [], |r| r.get(0)).unwrap();
        assert_eq!(rows, 0, "{name} has rows after the chain");
    }
    out.push_str("-- seeds\n");
    let mut seeds = conn.prepare("SELECT key, value FROM meta ORDER BY key").unwrap();
    let rows: Vec<(String, Value)> =
        seeds.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(Result::unwrap).collect();
    for (key, value) in rows {
        // `bring_to` seeds the path rules the catalog is made on; `catalog_id` is a fresh random id per catalog.
        if key == "path_rules" {
            continue;
        }
        let literal = match (key.as_str(), value) {
            ("catalog_id", _) => "lower(hex(randomblob(16)))".to_string(),
            (_, Value::Integer(i)) => i.to_string(),
            (_, Value::Text(t)) => format!("'{}'", t.replace('\'', "''")),
            (_, other) => panic!("seed {key} has an unexpected value {other:?}"),
        };
        writeln!(out, "INSERT INTO meta(key, value) VALUES ('{key}', {literal});").unwrap();
        println!("seed {key}");
    }
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join("baseline.sql");
    std::fs::write(&path, out).unwrap();
    println!("wrote {}", path.display());
}

/// The differential gate (2.3): the baked baseline and the migration chain make the same schema. It runs while both
/// lineages exist; the chain is deleted after it passes.
#[cfg(test)]
mod gate {
    use super::BAKED;
    use crate::schema::{BASELINE_VERSION, Hooks, SCHEMA, SETTINGS, SchemaSet, bring_to, current_version};
    use pg_platform::native::PATH_RULES;
    use pg_testkit::run_folder;
    use rusqlite::Connection;
    use rusqlite::types::Value;
    use std::path::PathBuf;

    const SCRATCH: &str = "scratch_bake";
    const BAKED_SET: SchemaSet<'static> = SchemaSet { baseline: BAKED, migrations: &[] };

    /// A fresh catalog file made as `prepare` makes one: `auto_vacuum` INCREMENTAL before the schema, the settings after.
    fn made(set: &SchemaSet<'_>, target: i64, name: &str) -> (Connection, PathBuf) {
        let path = run_folder(SCRATCH, name).unwrap().join("catalog.sqlite");
        let conn = Connection::open(&path).unwrap();
        conn.pragma_update(None, "auto_vacuum", "INCREMENTAL").unwrap();
        bring_to(&conn, set, PATH_RULES, target, &Hooks::none()).unwrap();
        conn.execute_batch(SETTINGS).unwrap();
        (conn, path)
    }

    fn user_version(conn: &Connection) -> i64 {
        conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap()
    }

    /// One line per row of `sql` (bound to `name`): its cells, in the order the query gives them.
    fn rows(conn: &Connection, sql: &str, name: &str, out: &mut Vec<String>) {
        let mut stmt = conn.prepare(sql).unwrap();
        let width = stmt.column_count();
        let lines = stmt
            .query_map([name], |r| {
                let cells: rusqlite::Result<Vec<String>> =
                    (0..width).map(|i| Ok(format!("{:?}", r.get::<_, Value>(i)?))).collect();
                Ok(cells?.join(" | "))
            })
            .unwrap();
        for line in lines {
            out.push(format!("  {}", line.unwrap()));
        }
    }

    /// The schema as the gate compares it: the header fields a catalog keeps, every object's text and its columns, keys
    /// and indexes (by name), the triggers in the order SQLite fires them (by rowid), and `meta`'s keys and values
    /// (`catalog_id` is random per catalog, so only its key is compared). `user_version` is not in it: that is the one
    /// difference the gate allows.
    fn structure(conn: &Connection) -> Vec<String> {
        let mut out = Vec::new();
        for pragma in ["page_size", "auto_vacuum", "encoding", "application_id", "journal_mode"] {
            let value: Value = conn.query_row(&format!("PRAGMA {pragma}"), [], |r| r.get(0)).unwrap();
            out.push(format!("{pragma} = {value:?}"));
        }
        let mut objects = conn
            .prepare("SELECT type, name, tbl_name, sql FROM sqlite_schema WHERE type <> 'trigger' ORDER BY type, name")
            .unwrap();
        let objects: Vec<(String, String, String, Option<String>)> = objects
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        for (kind, name, table, sql) in objects {
            out.push(format!("{kind} {name} on {table}: {}", sql.as_deref().unwrap_or("(no text)")));
            match kind.as_str() {
                "table" | "view" => {
                    rows(
                        conn,
                        "SELECT cid, name, type, \"notnull\", dflt_value, pk, hidden FROM pragma_table_xinfo(?1)",
                        &name,
                        &mut out,
                    );
                    if kind == "table" {
                        rows(
                            conn,
                            "SELECT id, seq, \"table\", \"from\", \"to\", on_update, on_delete, \"match\" FROM pragma_foreign_key_list(?1)",
                            &name,
                            &mut out,
                        );
                    }
                }
                "index" => {
                    rows(conn, "SELECT seqno, cid, name, desc, coll, key FROM pragma_index_xinfo(?1)", &name, &mut out)
                }
                _ => {}
            }
        }
        let mut triggers = conn
            .prepare("SELECT name, tbl_name, sql FROM sqlite_schema WHERE type = 'trigger' ORDER BY rowid")
            .unwrap();
        let triggers: Vec<(String, String, String)> =
            triggers.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap().map(Result::unwrap).collect();
        for (order, (name, table, sql)) in triggers.into_iter().enumerate() {
            out.push(format!("trigger #{order} {name} on {table}: {sql}"));
        }
        let mut meta = conn.prepare("SELECT key, value FROM meta ORDER BY key").unwrap();
        let meta: Vec<(String, Value)> =
            meta.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(Result::unwrap).collect();
        for (key, value) in meta {
            let value = if key == "catalog_id" { "<random>".to_string() } else { format!("{value:?}") };
            out.push(format!("meta {key} = {value}"));
        }
        out
    }

    #[test]
    fn the_baked_baseline_makes_the_chains_schema_at_version_20() {
        let (chain, _) = made(&SCHEMA, current_version().unwrap(), "chain");
        let (baked, _) = made(&BAKED_SET, BASELINE_VERSION, "baked");
        // The one allowed difference: the chain's version counts its migrations, the baseline is version 1.
        assert_eq!(user_version(&chain), current_version().unwrap());
        assert_eq!(user_version(&baked), BASELINE_VERSION);
        let (want, got) = (structure(&chain), structure(&baked));
        // The comparison has teeth: it sees the whole schema, and it tells version 20 from the one before it.
        assert!(want.len() > 300, "the structure is only {} lines", want.len());
        let (before, _) = made(&SCHEMA, current_version().unwrap() - 1, "before");
        assert_ne!(structure(&before), want, "version 19 compares equal to version 20");
        let first = want.iter().zip(&got).position(|(a, b)| a != b);
        assert!(
            want == got,
            "the baked baseline differs from the chain at line {:?}:\n chain: {:?}\n baked: {:?}\n({} vs {} lines)",
            first,
            first.map(|i| want[i].clone()),
            first.map(|i| got.get(i).cloned()),
            want.len(),
            got.len(),
        );
    }

    /// A populated workload, the same statements on both files: inserts, updates, deletes and cascades through the tables,
    /// the views and the trigger families the baseline has (the stamps, the failures and their hidden flag, the pairing
    /// epoch, the notices' bound). Its contents are compared table by table.
    const WORKLOAD: &str = r#"
INSERT INTO roots(path, added) VALUES (X'2F7270', 100), (X'2F7332', 101);
INSERT INTO folders(path, key) VALUES (X'2F7270', X'01'), (X'2F7270' || X'2F61', X'02'), (X'2F7332', X'03');
INSERT INTO files(volume, file_ref, created, folder_id, name, ext, kind, size, mtime, order_key) VALUES
    (1, 10, 0, 1, X'612E6A7067', 'jpg', 'image', 1000, 500, X'01'),
    (1, 11, 0, 2, X'622E706E67', 'png', 'image', 2000, 600, X'02'),
    (1, 12, 0, 2, X'632E6D7034', 'mp4', 'video', 3000, 700, X'03');
INSERT INTO scans(root, started, finished) VALUES (X'2F7270', 100, 110);
INSERT INTO scan_leases(id, root, holder, heartbeat) VALUES (1, X'2F7270', 'app', 130);
INSERT INTO analysis(file_id, version, size, mtime, rotation, kind, analyzed, error, file_hash, pixel_hash, format,
    orientation, width, height, pdq_quality, pdq_dihedral, frames, taken, taken_utc, duration_ms, poster_ms,
    cut_left, cut_top, cut_right, cut_bottom, borders_due) VALUES
    (1, 1, 1000, 500, 0, 'image', 200, NULL, X'AA', X'BB', 'jpeg', 1, 4000, 3000, 50, zeroblob(32), 1, 1000000,
        NULL, NULL, NULL, 0, 0, 0, 0, 0),
    (2, 1, 2000, 600, 0, 'image', 201, NULL, X'CC', X'DD', 'png', 1, 800, 600, 40, zeroblob(32), 1, 1000001,
        NULL, NULL, NULL, 0, 0, 0, 0, 1),
    (3, 1, 3000, 700, 0, 'video', 202, 'decode failed', NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, 1, NULL,
        NULL, 4000, 1500, NULL, NULL, NULL, NULL, 0);
UPDATE analysis SET cut_left = 5, width = 3990 WHERE file_id = 1;
UPDATE analysis SET poster_ms = 1600 WHERE file_id = 3;
UPDATE files SET rotation = 1 WHERE id = 2;
UPDATE files SET missing = 1 WHERE id = 3;
INSERT INTO pdq_pairs(a, b, distance) VALUES (1, 2, 3);
INSERT INTO pairs_owed(file_id) VALUES (1);
INSERT INTO hashes_owed(file_id) VALUES (2);
INSERT INTO border_marks(file_id, marked) VALUES (1, 400);
INSERT INTO border_sides(file_id, pass, edge, kind, class, depth, color, straight, dev, drift, rejected)
    VALUES (1, 0, 'top', 'solid', 'black', 4, 0, 0.5, 0.1, NULL, NULL);
UPDATE border_sides SET rejected = 'flat' WHERE file_id = 1;
INSERT INTO ratings(file_id, rating, updated) VALUES (1, 4, 300);
UPDATE ratings SET rating = 5, updated = 301 WHERE file_id = 1;
INSERT INTO file_ratings(file_id, value) VALUES (2, 3);
INSERT INTO placeholders(file_id, size, mtime, rotation, version, hash) VALUES (2, 2000, 600, 0, 1, X'EE');
UPDATE placeholders SET version = 2 WHERE file_id = 2;
INSERT INTO file_writes(file_id, kind, value, queued, attempts, error, temp, temp_value) VALUES
    (1, 'rating', 3, 500, 0, NULL, NULL, NULL),
    (2, 'rotation', 1, 501, 1, 'busy', X'2E7431', 2);
INSERT INTO file_failures(file_id, pass, mtime, size, rotation, error, retry, first_seen, hidden)
    VALUES (2, 'thumbs', 600, 2000, 0, 'bad', 0, 100, 0);
INSERT INTO folder_failures(root, path, reason, error, walk, serial, first_seen)
    VALUES (1, X'2F7270', 'unreadable', 'denied', 1, 0, 100);
INSERT INTO root_removals(path, started, deferred) VALUES (X'2F7333', 120, 0);
INSERT INTO notices(key, part, root, level, source, kind, data, created, updated)
    VALUES ('keyed', NULL, 1, 1, 'scan', 'x', '{}', 1, 1);
INSERT INTO notices(key, level, source, kind, data, created, updated)
    WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 1005)
    SELECT NULL, 0, 'scan', 'x', '{}', i, i FROM n;
INSERT INTO ledger_ack(kind, seen_to, read, dismissed_to) VALUES ('thumbs', 3, NULL, NULL);
INSERT INTO session(id, folder, recursive, query, sort) VALUES (1, X'2F7270', 1, '{}', '{}');
INSERT INTO views(name, folder, recursive, query, sort) VALUES ('all', X'', 0, '{}', '{}');
DELETE FROM analysis WHERE file_id = 3;
DELETE FROM files WHERE id = 2;
DELETE FROM folder_failures WHERE root = 1 AND reason = 'unreadable';
DELETE FROM notices WHERE key = 'keyed';
"#;

    /// Every table and view's rows, sorted, one line each; `meta`'s random `catalog_id` is left out.
    fn contents(conn: &Connection) -> Vec<String> {
        let mut names =
            conn.prepare("SELECT name FROM sqlite_schema WHERE type IN ('table', 'view') ORDER BY name").unwrap();
        let names: Vec<String> = names.query_map([], |r| r.get(0)).unwrap().map(Result::unwrap).collect();
        let mut out = Vec::new();
        for name in names {
            let sql = if name == "meta" {
                "SELECT * FROM meta WHERE key <> 'catalog_id'".to_string()
            } else {
                format!("SELECT * FROM \"{name}\"")
            };
            let mut stmt = conn.prepare(&sql).unwrap();
            let width = stmt.column_count();
            let rows = stmt
                .query_map([], |r| {
                    let cells: rusqlite::Result<Vec<String>> =
                        (0..width).map(|i| Ok(format!("{:?}", r.get::<_, Value>(i)?))).collect();
                    Ok(cells?.join(" | "))
                })
                .unwrap();
            let mut rows: Vec<String> = rows.map(Result::unwrap).collect();
            rows.sort();
            out.push(format!("{name}: {} rows", rows.len()));
            out.extend(rows.into_iter().map(|row| format!("  {row}")));
        }
        out
    }

    #[test]
    fn the_populated_workload_leaves_the_same_contents_on_both() {
        let (chain, _) = made(&SCHEMA, current_version().unwrap(), "workload_chain");
        let (baked, _) = made(&BAKED_SET, BASELINE_VERSION, "workload_baked");
        chain.execute_batch(WORKLOAD).unwrap();
        baked.execute_batch(WORKLOAD).unwrap();
        let (want, got) = (contents(&chain), contents(&baked));
        assert!(want.iter().any(|line| line.starts_with("notices: ")), "the workload did not run");
        for summary in want.iter().filter(|line| !line.starts_with("  ")) {
            println!("{summary}");
        }
        assert_eq!(want, got);
    }

    fn normalized(text: &str) -> String {
        text.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    /// The macros the chain's DDL expands, each as its runtime statements expand it: the stored text must contain each
    /// expansion, so a change to a macro that one side keeps is seen here (both sides share the macro, so this checks the
    /// text the baseline froze against the macro as it stands).
    #[test]
    fn the_macros_in_the_stored_text_are_the_ones_the_code_uses() {
        let stored = normalized(BAKED);
        let expansions = [
            ("stale!", stale!()),
            ("next_seq!", next_seq!()),
            ("seq!", seq!()),
            ("next_pairing_epoch!", next_pairing_epoch!()),
            ("live_failures!", live_failures!()),
            ("kind_video!", pg_model::kind_video!()),
            ("write_rating!", pg_model::write_rating!()),
            ("write_rotation!", pg_model::write_rotation!()),
        ];
        for (name, text) in expansions {
            assert!(stored.contains(&normalized(text)), "{name} expands to {text:?}, not in the stored text");
        }
        // `kept_events!()` is a number in the notices trigger's LIMIT, not a fragment: asserted in that context.
        assert!(stored.contains(&format!("WHERE key IS NULL) - {}))", kept_events!())), "kept_events!() context");
        // `pairing_version!()` is an integer: its seed row is the stored text.
        assert!(stored.contains(&format!("'pairing_version', {})", pg_model::pairing_version!())));
    }
}
