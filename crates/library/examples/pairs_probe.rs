//! The pairing pass alone, for timing and for callgrind: `pairs_probe <folder>`, where the folder holds a `catalog.sqlite`
//! made on Windows. Only the pass is timed. The catalog is changed: give the probe a copy.
//!
//! - `PG_PROBE_WORKERS` sets the join's threads (default 1, so a callgrind run profiles one thread's work).
//! - `PG_PROBE_KEEP_OWED` set: the pass pairs what the catalog owes (`tools/profiling/pairing/set_owed.py` sets it).
//!   Unset: every eligible picture owes its pairs again first (`pairs_again`), the pass over the whole library; the
//!   catalog is then closed and opened again, so the pass does not merge that write's WAL or read its cached pages.
//! - `PG_PROBE_SLICE`, `PG_PROBE_CACHE_KIB`, `PG_PROBE_WRITE`, `PG_PROBE_LOAD_PARTS` and `PG_PROBE_FEW` set the
//!   pass's `Settings` (default `pairs::SLICE`, `pairs::CACHE_KIB`, `pairs::WRITE`, the load in as many parts as the
//!   join has threads, the join's few-owed threshold by thread count).
//! - `PG_PROBE_WIDTHS` sets the join's head words per stage, as 16 comma-separated counts (`Tuning::widths`), and
//!   `PG_PROBE_BLOCK` the query buckets of a scan task (`Tuning::block`).
//! - `PG_PROBE_GUARD` set: every write reads its pictures' analyses (`Settings::guard`), as after a change since the
//!   load; `guarded` in the line counts the writes that did.
//!
//! One line: what was owed, what was paired, the wall clock, where it went (`PairStats::timings`: `store` building the
//! join's store, `join` the joins after it, `prepare` on one thread; `sort`, `scan`, `starved` and `paused` the join's
//! threads' time, summed), and the pass's CPU cycles over all its threads (`cycles`, millions; 0 where the platform
//! does not count them).

use std::io::Write;
use std::path::PathBuf;
use std::time::Instant;

/// The probe's result: any error of the crates it uses.
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
use pg_catalog::Catalog;
use pg_jobs::control::Control;
use pg_jobs::pipeline::Options;
use pg_model::rules::PathRules;
use pg_platform::priority::Priority;

/// The environment variable `name` parsed, when it is set and parses.
fn env<T: std::str::FromStr>(name: &str) -> Option<T> {
    std::env::var(name).ok().and_then(|v| v.parse().ok())
}

fn main() -> Result<()> {
    let folder = PathBuf::from(std::env::args().nth(1).ok_or("usage: pairs_probe <folder with catalog.sqlite>")?);
    let workers = env(pg_testkit::inputs::PROBE_WORKERS).unwrap_or(1);
    let keep = std::env::var_os(pg_testkit::inputs::PROBE_KEEP_OWED).is_some();
    let mut settings = pg_import::pairs::Settings::default();
    settings.slice = env(pg_testkit::inputs::PROBE_SLICE).unwrap_or(settings.slice);
    settings.cache_kib = env(pg_testkit::inputs::PROBE_CACHE_KIB).unwrap_or(settings.cache_kib);
    settings.write = env(pg_testkit::inputs::PROBE_WRITE).unwrap_or(settings.write);
    settings.load_parts = env(pg_testkit::inputs::PROBE_LOAD_PARTS).or(settings.load_parts);
    settings.join.few_entries = env(pg_testkit::inputs::PROBE_FEW).or(settings.join.few_entries);
    settings.join.block = env(pg_testkit::inputs::PROBE_BLOCK).or(settings.join.block);
    settings.guard = std::env::var_os(pg_testkit::inputs::PROBE_GUARD).is_some();
    // Each stage's head words, as 16 comma-separated counts.
    if let Ok(list) = std::env::var(pg_testkit::inputs::PROBE_WIDTHS) {
        let widths: Vec<usize> = list.split(',').map(|w| w.trim().parse()).collect::<std::result::Result<_, _>>()?;
        let widths: [usize; 16] = widths.try_into().map_err(|w: Vec<usize>| format!("16 widths, not {}", w.len()))?;
        settings.join.widths = Some(widths);
    }
    let path = folder.join("catalog.sqlite");
    let mut catalog = Catalog::prepare(&path, pg_library::versions(), PathRules::Windows)?;
    let owed = if keep {
        catalog.owed_pairs()?
    } else {
        // Closed and opened again, so the pass's reads do not find the pages `pairs_again` read already cached. Its WAL
        // is merged by `pairs_again` itself.
        let owed = catalog.pairs_again()?;
        catalog.close()?;
        catalog = Catalog::open(&path, PathRules::Windows)?;
        owed
    };
    let mut opts = Options::new(Priority::Normal);
    opts.workers = workers;
    let ctl = Control::new();
    let cycles_before = pg_platform::clock::process_cycles();
    let start = Instant::now();
    let stats = pg_import::pairs::pair_with(&mut catalog, &opts, settings, &ctl)?;
    let seconds = start.elapsed().as_secs_f64();
    // The pass's CPU cycles over every thread (0 where they are not counted): its work, which other processes on the
    // machine do not inflate the way they do the wall clock.
    let cycles = match (cycles_before, pg_platform::clock::process_cycles()) {
        (Some(a), Some(b)) => b.saturating_sub(a),
        _ => 0,
    };
    let cycles = pg_model::num::millions(pg_model::num::size(cycles));
    let t = stats.timings;
    let ms = |d: std::time::Duration| d.as_secs_f64() * 1000.0;
    // What the join ran with: its tuning resolved for the pass's threads, as the join resolves it.
    let few = settings.join.few_entries_on(workers);
    writeln!(
        std::io::stdout(),
        "owed {owed} analyses; paired {} files, {} pairs in {seconds:.3} s (workers {workers}, slice {}, cache {} KiB, \
         few {}, complete {}, guarded {}) | load {:.0} store {:.0} join {:.0} prepare {:.0} sort {:.0} scan {:.0} \
         starved {:.0} paused {:.0} write {:.0} commit {:.0} ms | cycles {cycles}",
        stats.files,
        stats.pairs,
        settings.slice,
        settings.cache_kib,
        few,
        stats.complete,
        stats.guarded,
        ms(t.load),
        ms(t.store),
        ms(t.join),
        ms(t.joins.prepare),
        ms(t.joins.sort),
        ms(t.joins.scan),
        ms(t.joins.starved),
        ms(t.joins.paused),
        ms(t.write),
        ms(t.commit),
    )?;
    Ok(())
}
