//! Where the pairing pass's load goes, part by part: `pairs_load_probe <catalog.sqlite> [runs] [variant...]` runs each
//! variant (or those named) `runs` times (6 by default), in turn, and prints per variant the median of its wall time
//! and of its CPU cycles (the process's; the probe runs one thread), and the rows it read.
//!
//! The catalog is a copy of a real one, copied again into a scratch folder (nothing is written). Every variant but
//! `cached` opens a fresh read-only connection with the pass's cache (`pg_import::pairs::CACHE_KIB`), as
//! `Catalog::reader` does, so SQLite's cache starts empty and the pages come from the OS's file cache, as they do for a
//! pass after an import:
//!
//! - `today`: the pass's load as built (`Catalog::reader` and `Reader::pairing_work`), the reader's open and close
//!   included;
//! - `split`: the same with `analysis` read in parts on connections of their own (`Reader::pairing_work_split`), as
//!   many as `PG_PROBE_WORKERS` (default: the logical cores);
//! - `open`: a reader's connection opened and closed, no read;
//! - `count`: the load's first read, `count(*)` of `analysis`;
//! - `step`: the statement the load read before step 8 (`PAIRING_WORK`), each row stepped and no column read;
//! - `columns`: the same, every column read (the blob in place), no hash decoded and nothing kept;
//! - `cached`: `columns` on a connection whose cache holds the whole catalog, read once into it first (untimed): the
//!   statement without fetching its pages;
//! - `analysis`: `analysis` alone, scanned with the quality filter, file id, blob and quality read;
//! - `files-sets`: what step 8 would read from `files` instead of a lookup per row: the displaced ids, the present ids
//!   and the present pending ids, each from an index alone;
//! - `identity`: the ids of every file with an identity (`volume IS NOT NULL`), from `UNIQUE(volume, file_ref)`'s
//!   index alone: the analyses with a file, displaced ones left out;
//! - `analysis-cached`: `analysis` on a warm connection, as `cached`;
//! - `owed`: `pairs_owed`'s ids, in order.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use pg_import::pairs::CACHE_KIB;
use pg_model::rules::PathRules;
use rusqlite::{Connection, OpenFlags};

/// The load's one statement before the pairing plan's second round, step 8 (`work.rs` at `459c8e0`): each analysis
/// joined to its file and its owed row.
const PAIRING_WORK: &str =
    "SELECT a.file_id, o.file_id IS NOT NULL AND f.missing = 0 AND f.pending = 0, a.pdq_dihedral,
         f.missing = 0 AND f.pending = 0, a.pdq_quality
     FROM analysis a CROSS JOIN files f LEFT JOIN pairs_owed o ON o.file_id = a.file_id
     WHERE f.id = a.file_id AND f.volume IS NOT NULL AND a.pdq_quality >= ?1
     ORDER BY a.file_id";
const ANALYSIS: &str =
    "SELECT file_id, pdq_dihedral, pdq_quality FROM analysis WHERE pdq_quality >= ?1 ORDER BY file_id";
const DISPLACED: &str = "SELECT id FROM files WHERE volume IS NULL";
const IDENTITY: &str = "SELECT id FROM files WHERE volume IS NOT NULL";
const PRESENT: &str = "SELECT id FROM files INDEXED BY files_present WHERE missing = 0";
const PENDING: &str = "SELECT id FROM files INDEXED BY files_to_import WHERE missing = 0 AND pending = 1";
const OWED: &str = "SELECT file_id FROM pairs_owed ORDER BY file_id";

const VARIANTS: [&str; 12] = [
    "today",
    "split",
    "open",
    "count",
    "step",
    "columns",
    "cached",
    "analysis",
    "analysis-cached",
    "files-sets",
    "identity",
    "owed",
];

/// One run of a variant: its wall time, its cycles, the rows it read.
type Run = (Duration, u64, usize);

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(source) = args.first() else { bail!("usage: pairs_load_probe <catalog.sqlite> [runs] [variant...]") };
    let runs: usize = args.get(1).map_or(Ok(6), |r| r.parse()).context("runs")?;
    let chosen: Vec<&str> = args.iter().skip(2).map(String::as_str).collect();
    let variants: Vec<&str> = if chosen.is_empty() { VARIANTS.to_vec() } else { chosen };
    for v in &variants {
        ensure!(VARIANTS.contains(v), "no variant {v}");
    }
    let dir = pg_testkit::run_folder("scratch_catalog", "pairs-load-probe")?;
    let db = dir.join("catalog.sqlite");
    std::fs::copy(source, &db).context("copy the catalog")?;
    show_plans(&open(&db, CACHE_KIB)?)?;
    // The pass opens it as the app's catalog: held open (untimed) while the variants run, as during a pass.
    let catalog = pg_catalog::Catalog::open(&db, PathRules::Windows)?;

    let mut times: BTreeMap<&str, Vec<Run>> = BTreeMap::new();
    for run in 0..runs {
        // Each run starts at another variant, so no variant always follows the same one.
        for k in 0..variants.len() {
            let Some(&variant) = variants.get((k + run) % variants.len()) else { continue };
            times.entry(variant).or_default().push(timed(&catalog, &db, variant)?);
        }
    }
    drop(catalog);
    let mut out = std::io::stdout().lock();
    writeln!(out, "variant: wall (ms), cycles (M), rows; median of {runs} runs")?;
    for variant in &variants {
        let Some(t) = times.get(variant) else { continue };
        let wall = median(t.iter().map(|x| x.0.as_secs_f64() * 1e3).collect());
        let cycles = median(t.iter().map(|x| pg_model::num::size(x.1) / 1e6).collect());
        let rows = t.first().map_or(0, |x| x.2);
        writeln!(out, "{variant}: {wall:.1} ms, {cycles:.0} M; {rows} rows")?;
    }
    drop(out);
    std::fs::remove_dir_all(&dir)?;
    Ok(())
}

/// A read-only connection with a cache of `kib` KiB, as `Catalog::reader` opens one.
fn open(db: &Path, kib: u32) -> Result<Connection> {
    let conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
    conn.busy_timeout(Duration::from_secs(5))?;
    conn.pragma_update(None, "cache_size", -i64::from(kib))?;
    Ok(conn)
}

/// Each statement's plan.
fn show_plans(conn: &Connection) -> Result<()> {
    let mut out = std::io::stdout().lock();
    for sql in [PAIRING_WORK, ANALYSIS, DISPLACED, IDENTITY, PRESENT, PENDING, OWED] {
        let mut stmt = conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}"))?;
        let plan: Vec<String> = if sql.contains("?1") {
            stmt.query_map([pg_model::pdq::MIN_QUALITY], |r| r.get::<_, String>(3))?.collect::<rusqlite::Result<_>>()?
        } else {
            stmt.query_map([], |r| r.get::<_, String>(3))?.collect::<rusqlite::Result<_>>()?
        };
        let head: String = sql.split_whitespace().take(4).collect::<Vec<_>>().join(" ");
        writeln!(out, "plan of {head}: {}", plan.join("; "))?;
    }
    Ok(())
}

/// Runs `variant` once: its wall time, its cycles, the rows it read.
fn timed(catalog: &pg_catalog::Catalog, db: &Path, variant: &str) -> Result<Run> {
    // The cached variant's connection is filled before the clock starts.
    let warm = if variant.ends_with("cached") {
        let conn = open(db, 1 << 20)?;
        read_rows(&conn, if variant == "cached" { PAIRING_WORK } else { ANALYSIS }, true, true)?;
        Some(conn)
    } else {
        None
    };
    let cycles = pg_platform::clock::process_cycles().unwrap_or(0);
    let start = Instant::now();
    let rows = match variant {
        "today" => {
            let work = catalog.reader(CACHE_KIB)?.pairing_work()?;
            work.ids.len() + usize::try_from(work.legacy).unwrap_or(usize::MAX)
        }
        "split" => {
            let threads = std::env::var("PG_PROBE_WORKERS")
                .ok()
                .and_then(|w| w.parse().ok())
                .or_else(|| std::thread::available_parallelism().ok().map(std::num::NonZero::get))
                .unwrap_or(1);
            let work = catalog.reader(CACHE_KIB)?.pairing_work_split(threads, &|| {})?;
            work.ids.len() + usize::try_from(work.legacy).unwrap_or(usize::MAX)
        }
        "open" => {
            drop(open(db, CACHE_KIB)?);
            0
        }
        "count" => open(db, CACHE_KIB)?.query_row("SELECT count(*) FROM analysis", [], |r| r.get(0))?,
        "step" => read_rows(&open(db, CACHE_KIB)?, PAIRING_WORK, true, false)?,
        "columns" => read_rows(&open(db, CACHE_KIB)?, PAIRING_WORK, true, true)?,
        "cached" => read_rows(warm.as_ref().context("the warm connection")?, PAIRING_WORK, true, true)?,
        "analysis" => read_rows(&open(db, CACHE_KIB)?, ANALYSIS, true, true)?,
        "analysis-cached" => read_rows(warm.as_ref().context("the warm connection")?, ANALYSIS, true, true)?,
        "identity" => read_rows(&open(db, CACHE_KIB)?, IDENTITY, false, true)?,
        "files-sets" => {
            let conn = open(db, CACHE_KIB)?;
            let mut rows = 0;
            for sql in [DISPLACED, PRESENT, PENDING] {
                rows += read_rows(&conn, sql, false, true)?;
            }
            rows
        }
        "owed" => read_rows(&open(db, CACHE_KIB)?, OWED, false, true)?,
        other => bail!("no variant {other}"),
    };
    let wall = start.elapsed();
    let cycles = pg_platform::clock::process_cycles().unwrap_or(0).saturating_sub(cycles);
    Ok((wall, cycles, rows))
}

/// Steps every row of `sql` in one read transaction (bound to the quality floor when `quality`), reading every column
/// in place when `columns`; returns the rows.
fn read_rows(conn: &Connection, sql: &str, quality: bool, columns: bool) -> Result<usize> {
    let tx = conn.unchecked_transaction()?;
    let mut stmt = tx.prepare(sql)?;
    let n = stmt.column_count();
    let mut rows = if quality { stmt.query([pg_model::pdq::MIN_QUALITY])? } else { stmt.query([])? };
    let (mut count, mut bytes) = (0usize, 0usize);
    while let Some(r) = rows.next()? {
        count += 1;
        if columns {
            for c in 0..n {
                bytes += match r.get_ref(c)? {
                    rusqlite::types::ValueRef::Blob(b) => b.len(),
                    _ => 1,
                };
            }
        }
    }
    std::hint::black_box(bytes);
    drop(rows);
    drop(stmt);
    tx.commit()?;
    Ok(count)
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    let n = v.len();
    match (v.get((n.max(1) - 1) / 2), v.get(n / 2)) {
        (Some(a), Some(b)) => (a + b) / 2.0,
        _ => 0.0,
    }
}
