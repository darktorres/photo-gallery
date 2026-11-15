//! The pairing pass alone, for callgrind: `pairs_probe <folder>`, where the folder holds a `catalog.sqlite` made on Windows.
//! Every eligible picture owes its pairs again (`pairs_again`), and the pass makes them; only the pass is timed. The
//! catalog is changed: give the probe a copy. `PG_PROBE_WORKERS` sets the join's threads (default 1, so a callgrind run
//! profiles one thread's work).

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
    let mut catalog = Catalog::prepare(&folder.join("catalog.sqlite"), pg_library::versions(), PathRules::Windows)?;
    let owed = catalog.pairs_again()?;
    let mut opts = Options::new(Priority::Normal);
    opts.workers = workers;
    let ctl = Control::new();
    let start = Instant::now();
    let stats = pg_import::pairs::pair(&mut catalog, &opts, &ctl)?;
    let seconds = start.elapsed().as_secs_f64();
    writeln!(
        std::io::stdout(),
        "owed {owed} analyses; paired {} files, {} pairs in {seconds:.3} s (workers {workers}, complete {})",
        stats.files,
        stats.pairs,
        stats.complete
    )?;
    Ok(())
}
