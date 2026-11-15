//! The pairing pass alone, for timing and for callgrind: `pairs_probe <folder>`, where the folder holds a `catalog.sqlite`
//! made on Windows. Only the pass is timed. The catalog is changed: give the probe a copy.
//!
//! - `PG_PROBE_WORKERS` sets the join's threads (default 1, so a callgrind run profiles one thread's work).
//! - `PG_PROBE_KEEP_OWED` set: the pass pairs what the catalog owes (`tools/profiling/pairing/set_owed.py` sets it).
//!   Unset: every eligible picture owes its pairs again first (`pairs_again`), the pass over the whole library; the
//!   catalog is then closed and opened again, so the pass does not merge that write's WAL or read its cached pages.
//! - `PG_PROBE_SLICE` and `PG_PROBE_CACHE_KIB` set the pass's `Settings` (default `pairs::SLICE`, `pairs::CACHE_KIB`).
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

fn main() -> Result<()> {
    let folder = PathBuf::from(std::env::args().nth(1).context("usage: pairs_probe <folder with catalog.sqlite>")?);
    let workers = std::env::var("PG_PROBE_WORKERS").ok().and_then(|v| v.parse().ok()).unwrap_or(1);
    let keep = std::env::var_os("PG_PROBE_KEEP_OWED").is_some();
    let mut settings = pg_import::pairs::Settings::default();
    if let Some(slice) = std::env::var("PG_PROBE_SLICE").ok().and_then(|v| v.parse().ok()) {
        settings.slice = slice;
    }
    if let Some(kib) = std::env::var("PG_PROBE_CACHE_KIB").ok().and_then(|v| v.parse().ok()) {
        settings.cache_kib = kib;
    }
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
    writeln!(
        std::io::stdout(),
        "owed {owed} analyses; paired {} files, {} pairs in {seconds:.3} s (workers {workers}, slice {}, cache {} KiB, \
         complete {}) | load {:.0} join {:.0} write {:.0} commit {:.0} ms",
        stats.files,
        stats.pairs,
        settings.slice,
        settings.cache_kib,
        stats.complete,
        ms(t.load),
        ms(t.join),
        ms(t.write),
        ms(t.commit),
    )?;
    Ok(())
}
