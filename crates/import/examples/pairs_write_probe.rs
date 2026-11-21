//! Where the pairing pass's write goes, part by part: `pairs_write_probe <catalog.sqlite> <pairs.txt> [runs]
//! [variant...]` runs each variant (or those named) `runs` times (6 by default), in turn, and prints per variant the
//! median of its statements' time, its commits' time, and the WAL's size after it.
//!
//! The catalog is a copy of a real one (it is written to); `pairs.txt` is the pairs the pass makes on it, one
//! `a b distance` per line (`a < b`, by file id). Before each run of a variant, every picture owes its pairs and no pair
//! is stored (`pairs_owed` holds every analysis the pass pairs, `pdq_pairs` is empty), the WAL is checkpointed and
//! truncated, and a fresh connection is opened with the catalog's settings and the pass's cache (`CACHE_KIB`). The
//! pictures and their stored hashes are read as the pass's load reads them, untimed. Then the variant writes every
//! picture's pairs and marks it done, in transactions of `WRITE` owed pictures in file-id order, as `write_slice` does
//! (every picture owed, so a pair's owner is its `b`):
//!
//! - `today`: `PairsWriter`'s statements (each pair and each done guarded by its analyses' stored hashes);
//! - `today-nockpt`: the same with `wal_autocheckpoint = 0`: the WAL's whole size, and the commits without checkpoints;
//! - `bare`: the same writes with no guard;
//! - `reads`: the guards' reads alone (`SELECT pdq_dihedral IS ?2`), and no write;
//! - `reads-cached`: the same, with a cache that holds the whole catalog and every `analysis` page read into it first
//!   (untimed): what the reads cost without fetching the pages;
//! - `noop`: one `SELECT` of the statement's parameters per statement: the cost of a statement itself;
//! - `empty`: the transactions alone;
//! - `set`: the set-based guard: one read of the batch's owners and partners (`rarray`), compared here, the passing pairs
//!   inserted unguarded, and one delete of the passing owners (`rarray`);
//! - `set-read`, `set-delete`: that read alone, and that delete alone (every owner).

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use pg_import::pairs::{CACHE_KIB, WRITE};
use rusqlite::types::Value;
use rusqlite::{Connection, params};

/// The catalog's connection settings: a copy of `schema::SETTINGS`, which is crate-private, without its cache. The
/// pass's cache (`CACHE_KIB`, from `pg_import::pairs`) is set in `open`, after these.
const SETTINGS: &str = "PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL; PRAGMA foreign_keys = ON;
     PRAGMA temp_store = MEMORY; PRAGMA busy_timeout = 5000;";
/// `PairsWriter`'s statements.
const PAIR: &str = "INSERT INTO pdq_pairs(a, b, distance)
     SELECT ?1, ?2, ?3
     WHERE (SELECT pdq_dihedral FROM analysis WHERE file_id = ?1) IS ?4
       AND (SELECT pdq_dihedral FROM analysis WHERE file_id = ?2) IS ?5
     ON CONFLICT(a, b) DO UPDATE SET distance = excluded.distance WHERE excluded.distance < pdq_pairs.distance";
const DONE: &str = "DELETE FROM pairs_owed
     WHERE file_id = ?1 AND (SELECT pdq_dihedral FROM analysis WHERE file_id = ?1) IS ?2";
const BARE_PAIR: &str = "INSERT INTO pdq_pairs(a, b, distance) VALUES (?1, ?2, ?3)
     ON CONFLICT(a, b) DO UPDATE SET distance = excluded.distance WHERE excluded.distance < pdq_pairs.distance";
const BARE_DONE: &str = "DELETE FROM pairs_owed WHERE file_id = ?1";
const READ: &str = "SELECT pdq_dihedral IS ?2 FROM analysis WHERE file_id = ?1";
const SET_READ: &str = "SELECT file_id, pdq_dihedral, pdq_quality FROM analysis WHERE file_id IN rarray(?1)";
const SET_DELETE: &str = "DELETE FROM pairs_owed WHERE file_id IN rarray(?1)";

const VARIANTS: [&str; 10] =
    ["today", "today-nockpt", "bare", "reads", "reads-cached", "noop", "empty", "set", "set-read", "set-delete"];

/// A picture as the pass holds it: its file id, its stored hashes and quality, and the pairs it owns (the partner's
/// index, the distance).
struct Picture {
    id: i64,
    blob: Vec<u8>,
    quality: i64,
    owns: Vec<(usize, i64)>,
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(source), Some(pairs)) = (args.first(), args.get(1)) else {
        bail!("usage: pairs_write_probe <catalog.sqlite> <pairs.txt> [runs] [variant...]");
    };
    let runs: usize = args.get(2).map_or(Ok(6), |r| r.parse()).context("runs")?;
    let chosen: Vec<&str> = args.iter().skip(3).map(String::as_str).collect();
    let variants: Vec<&str> = if chosen.is_empty() { VARIANTS.to_vec() } else { chosen };
    for v in &variants {
        ensure!(VARIANTS.contains(v), "no variant {v}");
    }
    let dir = pg_testkit::run_folder("scratch_catalog", "pairs-write-probe")?;
    let db = dir.join("catalog.sqlite");
    std::fs::copy(source, &db).context("copy the catalog")?;

    let mut times: BTreeMap<&str, Vec<(Duration, Duration, u64)>> = BTreeMap::new();
    let mut plans_shown = false;
    for run in 0..runs {
        // Each run starts at another variant, so no variant always follows the same one.
        for k in 0..variants.len() {
            let Some(&variant) = variants.get((k + run) % variants.len()) else { continue };
            reset(&db)?;
            let conn = open(&db)?;
            if !plans_shown {
                show_plans(&conn)?;
                plans_shown = true;
            }
            let pictures = load(&conn, pairs)?;
            let (stmts, commits) = timed(&conn, &pictures, variant)?;
            let wal = std::fs::metadata(wal_of(&db)).map_or(0, |m| m.len());
            check(&conn, variant, &pictures)?;
            drop(conn);
            times.entry(variant).or_default().push((stmts, commits, wal));
        }
    }
    let mut out = std::io::stdout().lock();
    writeln!(out, "variant: statements, commits, total (ms), WAL after (MiB); median of {runs} runs")?;
    for variant in &variants {
        let Some(t) = times.get(variant) else { continue };
        let ms = |f: &dyn Fn(&(Duration, Duration, u64)) -> f64| median(t.iter().map(f).collect());
        let stmts = ms(&|x| x.0.as_secs_f64() * 1e3);
        let commits = ms(&|x| x.1.as_secs_f64() * 1e3);
        let total = ms(&|x| (x.0 + x.1).as_secs_f64() * 1e3);
        let wal = ms(&|x| f64::from(u32::try_from(x.2 >> 10).unwrap_or(u32::MAX)) / 1024.0);
        writeln!(out, "{variant}: {stmts:.1}, {commits:.1}, {total:.1}; WAL {wal:.1}")?;
    }
    drop(out);
    std::fs::remove_dir_all(&dir)?;
    Ok(())
}

fn wal_of(db: &Path) -> PathBuf {
    let mut name = db.as_os_str().to_owned();
    name.push("-wal");
    PathBuf::from(name)
}

fn open(db: &Path) -> Result<Connection> {
    let conn = Connection::open(db)?;
    conn.execute_batch(SETTINGS)?;
    conn.pragma_update(None, "cache_size", -i64::from(CACHE_KIB))?;
    rusqlite::vtab::array::load_module(&conn)?;
    Ok(conn)
}

/// Every picture owes its pairs, none is stored, and the WAL is empty.
fn reset(db: &Path) -> Result<()> {
    let conn = open(db)?;
    conn.execute_batch("BEGIN IMMEDIATE; DELETE FROM pdq_pairs; DELETE FROM pairs_owed;")?;
    conn.execute(
        "INSERT INTO pairs_owed(file_id) SELECT a.file_id FROM analysis a CROSS JOIN files f
         WHERE f.id = a.file_id AND f.volume IS NOT NULL AND a.pdq_dihedral IS NOT NULL AND a.pdq_quality >= ?1
         ORDER BY a.file_id",
        [pg_model::pdq::MIN_QUALITY],
    )?;
    conn.execute_batch("COMMIT;")?;
    conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))?;
    Ok(())
}

/// The set statements' plans: each id of the array a seek by key.
fn show_plans(conn: &Connection) -> Result<()> {
    let ids = Rc::new(vec![Value::Integer(1)]);
    for sql in [SET_READ, SET_DELETE] {
        let mut stmt = conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}"))?;
        let plan: Vec<String> = stmt.query_map([&ids], |r| r.get::<_, String>(3))?.collect::<rusqlite::Result<_>>()?;
        let head: String = sql.split_whitespace().take(4).collect::<Vec<_>>().join(" ");
        writeln!(std::io::stdout(), "plan of {head}: {}", plan.join("; "))?;
    }
    Ok(())
}

/// The pictures in file-id order, as the pass's load reads them, with the pairs each owns (all owed: the pair's `b`).
fn load(conn: &Connection, pairs: &str) -> Result<Vec<Picture>> {
    let mut stmt = conn.prepare(
        "SELECT a.file_id, a.pdq_dihedral, a.pdq_quality FROM analysis a CROSS JOIN files f
         WHERE f.id = a.file_id AND f.volume IS NOT NULL AND a.pdq_dihedral IS NOT NULL AND a.pdq_quality >= ?1
         ORDER BY a.file_id",
    )?;
    let mut pictures: Vec<Picture> = stmt
        .query_map([pg_model::pdq::MIN_QUALITY], |r| {
            Ok(Picture { id: r.get(0)?, blob: r.get(1)?, quality: r.get(2)?, owns: Vec::new() })
        })?
        .collect::<rusqlite::Result<_>>()?;
    let index = |pictures: &[Picture], id: i64| pictures.binary_search_by_key(&id, |p| p.id).ok();
    for line in std::fs::read_to_string(pairs)?.lines().filter(|l| !l.is_empty()) {
        let parts: Vec<i64> = line.split_whitespace().map(str::parse).collect::<Result<_, _>>()?;
        let &[a, b, distance] = parts.as_slice() else { bail!("a pair line that is not `a b distance`: {line}") };
        let (Some(ia), Some(ib)) = (index(&pictures, a), index(&pictures, b)) else { bail!("a pair of no picture") };
        pictures.get_mut(ib).context("b")?.owns.push((ia, distance));
    }
    Ok(pictures)
}

/// Runs `variant` over every picture: its statements' time and its commits' time.
fn timed(conn: &Connection, pictures: &[Picture], variant: &str) -> Result<(Duration, Duration)> {
    if variant == "today-nockpt" {
        conn.execute_batch("PRAGMA wal_autocheckpoint = 0;")?;
    }
    if variant == "reads-cached" {
        conn.execute_batch("PRAGMA cache_size = -1048576;")?;
        conn.query_row("SELECT sum(length(pdq_dihedral)) FROM analysis", [], |_| Ok(()))?;
    }
    let (mut stmts, mut commits) = (Duration::ZERO, Duration::ZERO);
    for batch in pictures.chunks(WRITE) {
        let writing = Instant::now();
        conn.execute_batch("BEGIN IMMEDIATE")?;
        match variant {
            "today" | "today-nockpt" | "bare" => {
                let bare = variant == "bare";
                let mut pair = conn.prepare(if bare { BARE_PAIR } else { PAIR })?;
                let mut done = conn.prepare(if bare { BARE_DONE } else { DONE })?;
                for p in batch {
                    for &(partner, distance) in &p.owns {
                        let a = pictures.get(partner).context("partner")?;
                        if bare {
                            pair.execute(params![a.id, p.id, distance])?;
                        } else {
                            pair.execute(params![a.id, p.id, distance, a.blob, p.blob])?;
                        }
                    }
                    if bare {
                        done.execute([p.id])?;
                    } else {
                        done.execute(params![p.id, p.blob])?;
                    }
                }
            }
            "reads" | "reads-cached" => {
                let mut read = conn.prepare(READ)?;
                for p in batch {
                    for &(partner, _) in &p.owns {
                        let a = pictures.get(partner).context("partner")?;
                        read.query_row(params![a.id, a.blob], |r| r.get::<_, bool>(0))?;
                        read.query_row(params![p.id, p.blob], |r| r.get::<_, bool>(0))?;
                    }
                    read.query_row(params![p.id, p.blob], |r| r.get::<_, bool>(0))?;
                }
            }
            "noop" => {
                let mut pair = conn.prepare("SELECT ?1, ?2, ?3, ?4, ?5")?;
                let mut done = conn.prepare("SELECT ?1, ?2")?;
                for p in batch {
                    for &(partner, distance) in &p.owns {
                        let a = pictures.get(partner).context("partner")?;
                        pair.query_row(params![a.id, p.id, distance, a.blob, p.blob], |_| Ok(()))?;
                    }
                    done.query_row(params![p.id, p.blob], |_| Ok(()))?;
                }
            }
            "empty" => {}
            "set" | "set-read" => set_guard(conn, pictures, batch, variant == "set")?,
            "set-delete" => {
                let owners = Rc::new(batch.iter().map(|p| Value::Integer(p.id)).collect::<Vec<_>>());
                conn.prepare_cached(SET_DELETE)?.execute([owners])?;
            }
            other => bail!("no variant {other}"),
        }
        let committing = Instant::now();
        conn.execute_batch("COMMIT")?;
        commits += committing.elapsed();
        stmts += committing.duration_since(writing);
    }
    Ok((stmts, commits))
}

/// The set-based guard of one batch: its owners' and partners' analyses read at once; a picture passes when its stored
/// hashes and quality are those it was paired from. With `write`, the pairs of two passing pictures are inserted and the
/// passing owners deleted from `pairs_owed`, in one statement.
fn set_guard(conn: &Connection, pictures: &[Picture], batch: &[Picture], write: bool) -> Result<()> {
    let mut ids: Vec<i64> = batch.iter().map(|p| p.id).collect();
    for p in batch {
        for &(partner, _) in &p.owns {
            ids.push(pictures.get(partner).context("partner")?.id);
        }
    }
    ids.sort_unstable();
    ids.dedup();
    let array = Rc::new(ids.iter().map(|&id| Value::Integer(id)).collect::<Vec<_>>());
    let mut passing: Vec<i64> = Vec::with_capacity(ids.len());
    {
        let mut read = conn.prepare_cached(SET_READ)?;
        let mut rows = read.query([array])?;
        while let Some(r) = rows.next()? {
            let id: i64 = r.get(0)?;
            let Ok(at) = pictures.binary_search_by_key(&id, |p| p.id) else { continue };
            let p = pictures.get(at).context("picture")?;
            let blob = r.get_ref(1)?.as_blob_or_null()?;
            let quality: Option<i64> = r.get(2)?;
            if blob == Some(&p.blob[..]) && quality == Some(p.quality) {
                passing.push(id);
            }
        }
    }
    if !write {
        return Ok(());
    }
    passing.sort_unstable();
    let passes = |id: i64| passing.binary_search(&id).is_ok();
    let mut pair = conn.prepare_cached(BARE_PAIR)?;
    let mut owners = Vec::with_capacity(batch.len());
    for p in batch {
        for &(partner, distance) in &p.owns {
            let a = pictures.get(partner).context("partner")?;
            if passes(a.id) && passes(p.id) {
                pair.execute(params![a.id, p.id, distance])?;
            }
        }
        if passes(p.id) {
            owners.push(Value::Integer(p.id));
        }
    }
    conn.prepare_cached(SET_DELETE)?.execute([Rc::new(owners)])?;
    Ok(())
}

/// The writing variants leave every pair stored and nothing owed; the others leave every picture owed.
fn check(conn: &Connection, variant: &str, pictures: &[Picture]) -> Result<()> {
    let count = |sql: &str| -> Result<usize> { Ok(conn.query_row(sql, [], |r| r.get(0))?) };
    let (owed, pairs) = (count("SELECT count(*) FROM pairs_owed")?, count("SELECT count(*) FROM pdq_pairs")?);
    let expected_pairs: usize = pictures.iter().map(|p| p.owns.len()).sum();
    let wrote = matches!(variant, "today" | "today-nockpt" | "bare" | "set");
    let deleted = wrote || variant == "set-delete";
    let want = (if deleted { 0 } else { pictures.len() }, if wrote { expected_pairs } else { 0 });
    ensure!((owed, pairs) == want, "{variant}: {owed} owed and {pairs} pairs after it, not {want:?}");
    Ok(())
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    let n = v.len();
    match (v.get((n.max(1) - 1) / 2), v.get(n / 2)) {
        (Some(a), Some(b)) => (a + b) / 2.0,
        _ => 0.0,
    }
}
