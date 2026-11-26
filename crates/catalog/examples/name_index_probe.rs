//! Gate G1 of `docs/plans/CATALOG_SIZE_PLAN.md`: what the name index's key function costs against an index of the exact
//! bytes. `name_index_probe <catalog.sqlite>` copies a catalog of this layout (a carried one) and, on the copy, times:
//!
//! - building each index: `files_mn(mid, missing, path_name_key(name))`, the catalog's, and `(mid, missing, name)`;
//! - 100,000 inserts of file rows with one of the two indexes on the table (the other dropped), in one transaction
//!   rolled back;
//! - 200,000 pinned name lookups through each, the pairing's shape (the key's probe with the exact name as the
//!   residual, or the exact name alone), cycling over the catalog's own names.
//!
//! The bar (the plan's): the function's index no more than twice the exact one's, for each. The copy is deleted at the
//! end; the source is only read.

use std::io::Write as _;
use std::time::{Duration, Instant};

use pg_model::rules::PathRules;
use rusqlite::{Connection, params};

/// The probe's result: any error, its words kept as text.
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const INSERTS: i64 = 100_000;
const LOOKUPS: usize = 200_000;

/// The exact-bytes index the function's is compared with: the same columns, the name's bytes in the key's place.
const EXACT: &str = "CREATE INDEX probe_exact ON files(mid, missing, name)";
const KEYED: &str = "CREATE INDEX files_mn ON files(mid, missing, path_name_key(name))";

fn main() -> Result<()> {
    let Some(source) = std::env::args().nth(1) else { return Err("usage: name_index_probe <catalog.sqlite>".into()) };
    let dir = pg_testkit::run_folder("scratch_catalog", "name-index-probe")?;
    let db = dir.join("catalog.sqlite");
    std::fs::copy(&source, &db)?;
    // A WAL-mode catalog keeps its newest pages in its log until a checkpoint: the copy takes the log too.
    let log = format!("{source}-wal");
    if std::path::Path::new(&log).exists() {
        std::fs::copy(&log, format!("{}-wal", db.display()))?;
    }
    let conn = Connection::open(&db)?;
    pg_query::register_functions(&conn, rules_of(&conn)?)?;
    conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA cache_size = -262144; PRAGMA foreign_keys = OFF;")?;
    let mut out = std::io::stdout().lock();

    // The builds, each on a table without the other.
    conn.execute_batch("DROP INDEX files_mn")?;
    let exact_build = timed(|| Ok(conn.execute_batch(EXACT)?))?;
    conn.execute_batch("DROP INDEX probe_exact")?;
    let keyed_build = timed(|| Ok(conn.execute_batch(KEYED)?))?;
    writeln!(out, "build: keyed {} ms, exact {} ms", ms(keyed_build), ms(exact_build))?;

    // The inserts, each with its index alone, alternated twice: the first run reads the source rows from disk, so each
    // index's better run is the one compared.
    let (mut keyed_inserts, mut exact_inserts) = (Duration::MAX, Duration::MAX);
    for _ in 0..2 {
        keyed_inserts = keyed_inserts.min(inserts(&conn)?);
        conn.execute_batch(&format!("DROP INDEX files_mn; {EXACT}"))?;
        exact_inserts = exact_inserts.min(inserts(&conn)?);
        conn.execute_batch(&format!("DROP INDEX probe_exact; {KEYED}"))?;
    }
    writeln!(out, "{INSERTS} inserts: keyed {} ms, exact {} ms", ms(keyed_inserts), ms(exact_inserts))?;

    // The lookups, with both indexes there, each pinned, each run twice (the better run).
    conn.execute_batch(EXACT)?;
    let names: Vec<(i64, i64, Vec<u8>)> = conn
        .prepare("SELECT id, mid, name FROM files ORDER BY id")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;
    let keyed_sql = "SELECT id FROM files INDEXED BY files_mn WHERE mid = ?1 AND missing IN (0, 1)
         AND path_name_key(name) = path_name_key(?2) AND name = ?2 AND id <> ?3";
    let exact_sql = "SELECT id FROM files INDEXED BY probe_exact WHERE mid = ?1 AND missing IN (0, 1) AND name = ?2
         AND id <> ?3";
    let (mut keyed_lookups, mut exact_lookups) = (Duration::MAX, Duration::MAX);
    let (mut keyed_found, mut exact_found) = (0, 0);
    for _ in 0..2 {
        let (time, found) = lookups(&conn, keyed_sql, &names)?;
        (keyed_lookups, keyed_found) = (keyed_lookups.min(time), found);
        let (time, found) = lookups(&conn, exact_sql, &names)?;
        (exact_lookups, exact_found) = (exact_lookups.min(time), found);
    }
    writeln!(
        out,
        "{LOOKUPS} lookups: keyed {} ms ({keyed_found} other rows found), exact {} ms ({exact_found})",
        ms(keyed_lookups),
        ms(exact_lookups)
    )?;
    for (label, keyed, exact) in [
        ("build", keyed_build, exact_build),
        ("inserts", keyed_inserts, exact_inserts),
        ("lookups", keyed_lookups, exact_lookups),
    ] {
        let ratio = keyed.as_secs_f64() / exact.as_secs_f64().max(1e-9);
        writeln!(
            out,
            "{label}: {ratio:.2}x the exact index ({})",
            if ratio <= 2.0 { "within the bar" } else { "OVER the bar" }
        )?;
    }
    drop(out);
    drop(conn);
    std::fs::remove_dir_all(&dir)?;
    Ok(())
}

/// The rules the catalog records.
fn rules_of(conn: &Connection) -> Result<PathRules> {
    let name: String = conn.query_row("SELECT value FROM meta WHERE key = 'path_rules'", [], |r| r.get(0))?;
    Ok(if name == "posix" { PathRules::Posix } else { PathRules::Windows })
}

fn timed(f: impl FnOnce() -> Result<()>) -> Result<Duration> {
    let started = Instant::now();
    f()?;
    Ok(started.elapsed())
}

fn ms(d: Duration) -> u128 {
    d.as_millis()
}

/// `INSERTS` new rows, each a copy of an existing one (its folder, name and codes) with an identity no file has, in one
/// transaction that is rolled back.
fn inserts(conn: &Connection) -> Result<Duration> {
    conn.execute_batch("BEGIN")?;
    let elapsed = timed(|| {
        let mut insert = conn.prepare(
            "INSERT INTO files(volume, file_ref, folder_id, mid, name, ext, kind, size, mtime)
             SELECT volume, ?1, folder_id, mid, CAST(name || CAST(?1 AS TEXT) AS BLOB), ext, kind, size, mtime
             FROM files WHERE id = ?2",
        )?;
        let high: i64 = conn.query_row("SELECT max(id) FROM files", [], |r| r.get(0))?;
        for n in 0..INSERTS {
            insert.execute(params![(1_i64 << 60) + n, 1 + n % high])?;
        }
        Ok(())
    })?;
    conn.execute_batch("ROLLBACK")?;
    Ok(elapsed)
}

/// `LOOKUPS` runs of `sql` over the catalog's own names, cycling: the time, and the rows found (another row of the same
/// name in the same merged folder; none in a library without such names).
fn lookups(conn: &Connection, sql: &str, names: &[(i64, i64, Vec<u8>)]) -> Result<(Duration, usize)> {
    let mut stmt = conn.prepare(sql)?;
    let mut found = 0_usize;
    let elapsed = timed(|| {
        for (id, mid, name) in names.iter().cycle().take(LOOKUPS) {
            let mut rows = stmt.query(params![mid, name, id])?;
            while rows.next()?.is_some() {
                found += 1;
            }
        }
        Ok(())
    })?;
    Ok((elapsed, found))
}
