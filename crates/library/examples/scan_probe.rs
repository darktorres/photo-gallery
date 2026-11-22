//! The scan step alone, for callgrind: `scan_probe <folder>` adds the folder to a new catalog as the CLI adds it
//! (`folders::add_folder`), then walks it with `pg_scan::scan`, and only that walk is timed. The walk is the platform's
//! listing of the real folder, and its batches are written to a catalog file, as the app's are; its pairing and its
//! missing marks are in the timed call, since they are part of the scan step. `PG_PROBE_AGAIN` set walks the folder
//! once untimed first, so the timed walk is a rescan (every file seen, none changed) rather than a first scan; with it,
//! `PG_PROBE_BETWEEN` names a command run between the two walks (one that empties the file cache, for a cold rescan).
//!
//! The catalog is made fresh in a run folder (`pg_testkit::run_folder`: under the temp folder, or `PG_TEST_DIR`). Valgrind
//! is Linux-only, so run this in the container, on a folder it can read (a synthetic library from
//! tools/profiling/synthetic_library.py), e.g. `valgrind --tool=callgrind target/release/examples/scan_probe /bench/lib`.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// The probe's result: any error of the crates it uses.
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
use pg_catalog::Catalog;
use pg_jobs::control::Control;
use pg_library::folders::add_folder;
use pg_platform::native::PATH_RULES;

fn main() -> Result<()> {
    let folder = PathBuf::from(std::env::args_os().nth(1).ok_or("usage: scan_probe <library folder>")?);
    let again = std::env::var_os("PG_PROBE_AGAIN").is_some();
    let dir = pg_testkit::run_folder("scratch_library", "scan-probe")?;
    run(&folder, &dir.join("catalog.sqlite"), again)
}

fn run(folder: &Path, catalog_path: &Path, again: bool) -> Result<()> {
    let mut catalog = Catalog::prepare(catalog_path, pg_library::versions(), PATH_RULES)?;
    let (root, _) = add_folder(&catalog, folder)?;
    let ctl = Control::new();
    if again {
        pg_scan::scan(&mut catalog, &root, &ctl)?;
        // A command run between the two walks (`PG_PROBE_BETWEEN`, split on spaces): emptying the file cache there
        // makes the timed rescan a cold one.
        if let Some(command) = std::env::var_os("PG_PROBE_BETWEEN") {
            let command = command.to_string_lossy().into_owned();
            let mut parts = command.split_whitespace();
            let program = parts.next().ok_or("PG_PROBE_BETWEEN is empty")?;
            let status = std::process::Command::new(program).args(parts).status()?;
            if !status.success() {
                return Err(format!("PG_PROBE_BETWEEN failed: {status}").into());
            }
        }
    }
    let start = Instant::now();
    let stats = pg_scan::scan(&mut catalog, &root, &ctl)?;
    let seconds = start.elapsed().as_secs_f64();
    writeln!(
        std::io::stdout(),
        "scanned {} files in {} folders ({} changed, {} missing, {} folder errors) in {seconds:.3} s (complete {}, again {again})",
        stats.files,
        stats.folders,
        stats.changed,
        stats.missing,
        stats.errors,
        stats.complete,
    )?;
    Ok(())
}
