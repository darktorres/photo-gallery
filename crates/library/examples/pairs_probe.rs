//! The pairing pass alone, for timing and for callgrind: `pairs_probe <folder>`, where the folder holds a `catalog.sqlite`
//! made on Windows. Only the pass is timed. The catalog is changed: give the probe a copy.
//!
//! - `PG_PROBE_WORKERS` sets the join's threads (default 1, so a callgrind run profiles one thread's work).
//! - `PG_PROBE_KEEP_OWED` set: the pass pairs what the catalog owes (`tools/profiling/pairing/set_owed.py` sets it).
//!   Unset: every eligible picture owes its pairs again first (`pairs_again`), the pass over the whole library; the
//!   catalog is then closed and opened again, so the pass does not merge that write's WAL or read its cached pages.
//! - `PG_PROBE_SLICE`, `PG_PROBE_CACHE_KIB`, `PG_PROBE_SORTERS` and `PG_PROBE_FEW` set the pass's `Settings` (default
//!   `pairs::SLICE`, `pairs::CACHE_KIB`, the join's sorters and few-owed threshold by thread count).
//!
//! One line: what was owed, what was paired, the wall clock, and where it went (`PairStats::timings`).

use std::io::Write;
use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result};
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
    let folder = PathBuf::from(std::env::args().nth(1).context("usage: pairs_probe <folder with catalog.sqlite>")?);
    let workers = env("PG_PROBE_WORKERS").unwrap_or(1);
    let keep = std::env::var_os("PG_PROBE_KEEP_OWED").is_some();
    let mut settings = pg_import::pairs::Settings::default();
    settings.slice = env("PG_PROBE_SLICE").unwrap_or(settings.slice);
    settings.cache_kib = env("PG_PROBE_CACHE_KIB").unwrap_or(settings.cache_kib);
    settings.join.sorters = env("PG_PROBE_SORTERS").or(settings.join.sorters);
    settings.join.few_entries = env("PG_PROBE_FEW").or(settings.join.few_entries);
    let path = folder.join("catalog.sqlite");
    let mut catalog = Catalog::prepare(&path, pg_library::versions(), PathRules::Windows)?;
    let owed = if keep {
        catalog.owed_pairs()?
    } else {
        // Closed (its WAL merged) and opened again, so the pass is timed alone: neither the WAL `pairs_again` wrote,
        // which the pass's first commits would merge, nor the pages it read, which the pass's reads would find cached.
        let owed = catalog.pairs_again()?;
        catalog.close()?;
        catalog = Catalog::open(&path, PathRules::Windows)?;
        owed
    };
    let mut opts = Options::new(Priority::Normal);
    opts.workers = workers;
    let ctl = Control::new();
    let start = Instant::now();
    let stats = pg_import::pairs::pair_with(&mut catalog, &opts, settings, &ctl)?;
    let seconds = start.elapsed().as_secs_f64();
    let t = stats.timings;
    let ms = |d: std::time::Duration| d.as_secs_f64() * 1000.0;
    let sorters = settings.join.sorters.unwrap_or_else(|| pg_import::pairs::default_sorters(workers));
    writeln!(
        std::io::stdout(),
        "owed {owed} analyses; paired {} files, {} pairs in {seconds:.3} s (workers {workers}, sorters {sorters}, slice \
         {}, cache {} KiB, few {}, complete {}) | load {:.0} join {:.0} prepare {:.0} sort {:.0} scan {:.0} write {:.0} \
         commit {:.0} ms",
        stats.files,
        stats.pairs,
        settings.slice,
        settings.cache_kib,
        settings.join.few_entries.unwrap_or_else(|| pg_import::pairs::FEW_ENTRIES_PER_THREAD * workers),
        stats.complete,
        ms(t.load),
        ms(t.join),
        ms(t.joins.prepare),
        ms(t.joins.sort),
        ms(t.joins.scan),
        ms(t.write),
        ms(t.commit),
    )?;
    Ok(())
}
