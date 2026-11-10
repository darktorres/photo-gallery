//! The command line of `gallery.exe` (`gallery scan`, `gallery import`, …): the app's passes without the window,
//! for scripts, profiling and checking results. Progress goes to stderr.

// The command line's output is its product: its reports go to stdout, its progress and errors to stderr.
#![expect(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "the command line's reports and progress are its output"
)]

mod args;
mod borders;
mod dups;
mod report;

use pg_model::ids::FileId;
use pg_model::rules::{Path as RulePath, PathRules};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use pg_catalog::Catalog;
use pg_platform::priority::Priority;
use pg_thumbcache::ThumbCache;

use pg_jobs::control::Control;
use pg_jobs::events::Bus;
use pg_writeback::Writer;

use args::{Cli, Command, PipelineArgs};
use report::{changed_while_read, print_failures, report_rebuilt, thumb_events, with_progress};

/// Whether these arguments (without the program name) ask for the command line rather than the app.
/// Asked from the derive's list of commands (`Subcommand::has_subcommand`): clap's `Command` isn't built at an
/// ordinary start.
pub fn wants_cli(args: &[OsString]) -> bool {
    args.iter().any(|a| {
        a.to_str().is_some_and(|a| {
            matches!(a, "help" | "--help" | "-h" | "--version" | "-V") || <Command as Subcommand>::has_subcommand(a)
        })
    })
}

/// Run a command. `args` includes the program name.
pub fn main(args: impl IntoIterator<Item = OsString>) -> Result<()> {
    let cli = Cli::parse_from(args);
    // Borders of pixels piped in, or of files outside the library: no catalog.
    if let Command::Borders { paths, csv, stdin, pipeline, .. } = &cli.command
        && (*stdin || !paths.is_empty())
    {
        return borders_outside(paths, csv.as_deref(), *stdin, pipeline);
    }
    std::fs::create_dir_all(cli.layout()?.catalog_dir())?;
    with_progress(|ctl| run_command(cli, ctl))
}

/// `path` as a path under the catalog's rules, from its exact bytes; a path that is none is an error.
fn rule_path_of(rules: PathRules, path: &Path) -> Result<RulePath> {
    rules
        .parse_bytes(&pg_platform::names::canonical_of(path))
        .ok_or_else(|| anyhow::anyhow!("{} is not a path", path.display()))
}

fn run_command(cli: Cli, ctl: &Control) -> Result<()> {
    // Before the match below moves out of `cli.command`: the data folders are read from the whole `cli`.
    let layout = cli.layout()?;
    let catalog_dir = layout.catalog_dir().to_path_buf();
    let catalog_path = layout.catalog_path();
    let mut catalog = pg_library::open_catalog(&catalog_dir)?;
    report_rebuilt(&catalog);
    let rules = catalog.rules();
    let start = Instant::now();

    match cli.command {
        Command::Scan { roots } => {
            let roots: Vec<RulePath> = if roots.is_empty() {
                catalog.roots()?.into_iter().map(|r| r.path).collect()
            } else {
                let mut normalized = Vec::new();
                for root in roots {
                    let root = pg_scan::normalize_path(&root)?;
                    // A refused folder is not added; the other folders still are.
                    if let Err(refused) = pg_scan::check_folder(rules, &root) {
                        println!("{} is not added: {refused}", rules.display(&root));
                        continue;
                    }
                    if !catalog.add_root(&root)? {
                        println!("{} is already in the library", rules.display(&root));
                    }
                    normalized.push(root);
                }
                normalized
            };
            for root in roots {
                let name = rules.display(&root);
                let s = match pg_scan::scan(&mut catalog, &root, ctl) {
                    Ok(s) => s,
                    // A refused folder is reported and the others are still scanned; any other error stops the command.
                    Err(e) => match e.downcast_ref::<pg_platform::dir::Refused>() {
                        Some(refused) => {
                            println!("{name}: not scanned, {refused}");
                            continue;
                        }
                        None => return Err(e),
                    },
                };
                println!(
                    "{name}: {} files in {} folders, {} new or changed, {} now missing, {} unreadable folders ({:.1} s)",
                    s.files,
                    s.folders,
                    s.changed,
                    s.missing,
                    s.errors,
                    start.elapsed().as_secs_f64()
                );
                print_failures("skipped", &s.failures);
                // What the pairing replaced loses its thumbnails and the temp files of its queued writes (decision 7).
                if !s.replaced_ids.is_empty() {
                    let (events, printer) = thumb_events();
                    let cache = pg_library::open_thumbs(&catalog_dir, &catalog.catalog_id()?, None, events)?;
                    for id in &s.replaced_ids {
                        if let Err(e) = cache.forget(*id) {
                            println!("{name}: a replaced file's thumbnail wasn't deleted: {e:#}");
                        }
                    }
                    drop(cache);
                    let _ = printer.join();
                }
                for temp in &s.temps {
                    // By its disk path, as the runner does: the plain one could reach a sibling folder's file.
                    let disk = pg_platform::names::disk_path(rules, temp);
                    match std::fs::remove_file(&disk) {
                        Ok(()) => {}
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                        Err(e) => println!("{name}: can't delete {}: {e}", rules.display(temp)),
                    }
                }
                // The same notes the app shows, counted; a zero is not said.
                let to_u64 = |n: usize| u64::try_from(n).unwrap_or(u64::MAX);
                let notes = [
                    ("skipped as duplicates (another path to the same file)", s.duplicates),
                    ("indexed without following (links or placeholders)", s.skipped_reparse),
                    ("matched more than one earlier row, so nothing was carried over", to_u64(s.ambiguous)),
                    ("missing, with data and no file found for them", to_u64(s.unpaired_with_data)),
                    ("replaced by new files with the same name; content reset", to_u64(s.replaced)),
                    ("found under a new identity; their data was kept", to_u64(s.rebound)),
                ];
                for (what, count) in notes.iter().filter(|(_, count)| *count > 0) {
                    println!("  {count} {what}");
                }
            }
        }
        Command::Remove { root } => {
            // A folder deleted from disk can't be normalized, and it is the one to remove: its path as given is then
            // the catalog's, and the catalog says whether it is a library folder.
            let root = match pg_scan::normalize_path(&root) {
                Ok(path) => path,
                Err(_) => rule_path_of(rules, &root)?,
            };
            let text = rules.display(&root);
            let removed = match catalog.root_id(&root)? {
                Some(id) => catalog.remove_root(id)?,
                None => None,
            };
            match removed {
                Some(n) => println!("{text}: removed from the library, {n} files hidden"),
                None => anyhow::bail!("{text}: not in the library"),
            }
        }
        Command::Import { pipeline, retry_errors, again } => {
            if !again.is_empty() {
                let marked = catalog.mark_pending(&files_named(&catalog, &again)?)?;
                println!("{marked} files to analyze again");
            }
            let opts = pipeline.options(Priority::Normal);
            let s = pg_import::import(&mut catalog, &opts, retry_errors, ctl)?;
            println!("{} files to import; the tier-1 index held {} hashes", s.pending, s.indexed);
            println!(
                "imported {} files ({} failed{}), {} PDQ pairs, {:.1} s",
                s.files,
                s.failed,
                changed_while_read(s.changed),
                s.pairs,
                start.elapsed().as_secs_f64()
            );
        }
        Command::Thumbs { height, cap, pipeline, foreground } => {
            // Idle: only the cores the user leaves free. I/O priority stays normal: Windows' background mode
            // also throttles reads on an idle disk and made the task 6.5× slower (64 vs 415 files/s on dups2).
            let opts = pipeline.options(if foreground { Priority::Normal } else { Priority::Idle });
            let (events, printer) = thumb_events();
            let cache = pg_library::open_thumbs(&catalog_dir, &catalog.catalog_id()?, cap, events)?;
            let s = pg_thumbs::run(&cache, &catalog_path, height, &opts, ctl)?;
            cache.flush();
            println!(
                "{} files needed a {height} px thumbnail; the cache held {:.2} GB",
                s.needed,
                pg_model::num::size(s.cache_bytes) / 1e9
            );
            print_failures("failed", &s.failures);
            println!(
                "{} thumbnails at {height} px ({} failed{}){}; cache {:.2} of {:.2} GB, {:.1} s",
                s.made,
                s.failed,
                changed_while_read(s.changed),
                if s.cache_full { ", stopped: cache full" } else { "" },
                pg_model::num::signed(cache.totals()?.0) / 1e9,
                pg_model::num::signed(cache.cap()?) / 1e9,
                start.elapsed().as_secs_f64()
            );
            drop(cache);
            let _ = printer.join();
        }
        Command::Borders { import_review, pipeline, .. } => {
            if let Some(review) = import_review {
                let (marked, unknown) = borders::import_review(&catalog, &review)?;
                println!("{marked} files marked as not a border; {unknown} not in the catalog");
            } else {
                let s = pg_borders::find(&mut catalog, &pipeline.options(Priority::Normal), ctl)?;
                println!(
                    "borders looked for in {} files ({} failed, {} stored), {:.1} s",
                    s.files,
                    s.failed,
                    s.stored,
                    start.elapsed().as_secs_f64()
                );
            }
        }
        Command::Status => {
            println!("data folder: {}", layout.home().display());
            for root in catalog.roots()? {
                println!("library: {}", rules.display(&root.path));
            }
            let s = catalog.status()?;
            println!(
                "{} files ({} missing), {} analyzed, {} failed, {} pending, {} PDQ pairs",
                s.files, s.missing, s.analyzed, s.failed, s.pending, s.pdq_pairs
            );
            println!("{} changes to write into files ({} failing)", s.queued_writes, s.failed_writes);
            match ThumbCache::read_usage(&catalog_dir)? {
                Some(u) => println!(
                    "thumbnail cache: {} thumbnails, {:.2} of {:.2} GB",
                    u.count,
                    pg_model::num::signed(u.bytes) / 1e9,
                    pg_model::num::signed(u.cap) / 1e9
                ),
                None => println!("thumbnail cache: none yet"),
            }
            for e in pg_catalog::writes::recent_errors(&catalog, 5)? {
                println!("  {e}");
            }
        }
        Command::Write => {
            // Only retagged here (never filled): the cap stays as it is.
            // The cache is leaked, so its events print until the process ends.
            let (events, _printer) = thumb_events();
            let cache: &'static ThumbCache =
                Box::leak(Box::new(pg_library::open_thumbs(&catalog_dir, &catalog.catalog_id()?, None, events)?));
            let writer = Writer::start(catalog_path.clone(), cache, Arc::new(Bus::new(16)))?;
            writer.wake();
            wait_for_writes(&writer, &catalog)?;
            if let Some(e) = writer.status().thumbnail_error {
                eprintln!("thumbnails: {e}");
            }
            drop(writer);
            cache.close();
            let s = catalog.status()?;
            println!(
                "written; {} left ({} failing), {} files to re-import (turned), {:.1} s",
                s.queued_writes,
                s.failed_writes,
                s.pending,
                start.elapsed().as_secs_f64()
            );
        }
        Command::Dups { distance, show } => dups::report(&catalog, distance, show)?,
    }
    Ok(())
}

/// `gallery write`'s wait: until nothing writable is left (failures past their retries stay queued and are reported).
/// A writer that ended (a panic outside a job) leaves what it didn't write counted, so the wait fails then instead of
/// waiting for ever.
fn wait_for_writes(writer: &Writer, catalog: &Catalog) -> Result<()> {
    loop {
        std::thread::sleep(std::time::Duration::from_millis(500));
        let left = pg_catalog::writes::left_to_write(catalog)?;
        if left == 0 {
            return Ok(());
        }
        if !writer.is_running() {
            let why = writer.status().error.map(|e| format!(" (the last failure: {e})")).unwrap_or_default();
            bail!("the file writer stopped with {left} changes still to write{why}");
        }
    }
}

/// The present files of the catalog at these paths, each a file or a folder (its files and its subfolders'), each
/// once. Fails, before anything is done, on a path that holds none.
fn files_named(catalog: &Catalog, paths: &[PathBuf]) -> Result<Vec<FileId>> {
    let mut ids = Vec::new();
    for path in paths {
        let rule = pg_scan::normalize_path(path)?;
        let before = ids.len();
        if path.is_dir() {
            ids.extend(catalog.files_under(&rule)?);
        } else {
            ids.extend(catalog.file_by_path(&rule)?);
        }
        if ids.len() == before {
            bail!("{}: no file of the library there", path.display());
        }
    }
    ids.sort_unstable();
    ids.dedup();
    Ok(ids)
}

/// `gallery borders --stdin`, or `gallery borders --csv <out> <paths>`.
fn borders_outside(paths: &[PathBuf], csv: Option<&Path>, stdin: bool, pipeline: &PipelineArgs) -> Result<()> {
    if stdin {
        let n = borders::serve(std::io::stdin().lock(), std::io::stdout().lock())?;
        eprintln!("borders looked for in {n} images");
        return Ok(());
    }
    let csv = csv.context("--csv: where the report goes")?;
    let s = with_progress(|ctl| borders::report(paths, csv, &pipeline.options(Priority::Normal), ctl))?;
    println!(
        "{} files: {} with borders, {} rejected candidates only, {} skipped, {} errors; {:.1} s ({:.0} files/s)",
        s.files,
        s.borders,
        s.rejected,
        s.skipped,
        s.errors,
        s.seconds,
        pg_model::num::count(s.files) / s.seconds.max(1e-3)
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::*;

    #[test]
    fn the_command_line_is_told_from_the_app_without_building_clap() {
        let wants = |args: &[&str]| wants_cli(&args.iter().map(OsString::from).collect::<Vec<_>>());
        for command in Cli::command().get_subcommands() {
            assert!(wants(&[command.get_name()]), "{}", command.get_name());
        }
        for words in [&["help"][..], &["--help"], &["-h"], &["--version"], &["-V"], &["--catalog-dir", "x", "scan"]] {
            assert!(wants(words), "{words:?}");
        }
        for words in [&[][..], &["--screenshot=D:/x.png"], &["--view=3", "--viewer-keys=r3"], &["scanner"]] {
            assert!(!wants(words), "{words:?}");
        }
    }

    #[test]
    fn the_catalog_flag_is_catalog_dir() {
        let cli = Cli::try_parse_from(["gallery", "--catalog-dir", "X", "status"]).unwrap();
        assert_eq!(cli.layout().unwrap().catalog_dir(), std::path::Path::new("X"));
    }

    /// The writer panics outside a job (where it reads the queue), so its thread ends with a write still queued:
    /// `gallery write` returns an error instead of waiting for ever.
    #[test]
    fn gallery_write_returns_when_the_writer_ends() {
        use pg_catalog::fixture::state;
        use pg_writeback::fault::Fault;

        let dir = pg_testkit::run_folder_with("scratch_rate", "cli-write", &["images"]).unwrap();
        std::fs::write(dir.join("images").join("a.png"), pg_testkit::png_rgb(4, 4, &[0; 48]).unwrap()).unwrap();
        let catalog_path = dir.join("catalog.sqlite");
        let mut catalog =
            Catalog::prepare(&catalog_path, pg_library::versions(), pg_platform::native::PATH_RULES).unwrap();
        let root = pg_scan::normalize_path(&dir.join("images")).unwrap();
        pg_scan::scan(&mut catalog, &root, &Control::new()).unwrap();
        state::queue_write(&catalog, state::file_id(&catalog, "a.png").unwrap(), "rating", 3).unwrap();
        let (events, quiet) = pg_jobs_testkit::Quiet::bus();
        let thumbs: &'static ThumbCache =
            Box::leak(Box::new(ThumbCache::open(&dir, &catalog.catalog_id().unwrap(), Some(1 << 30), events).unwrap()));
        drop(catalog);
        let writer =
            Writer::start_with_fault(catalog_path.clone(), thumbs, Arc::new(Bus::new(16)), Fault::PanicInDrain)
                .unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let waited = Catalog::open(&catalog_path, pg_platform::native::PATH_RULES)
                .map_err(anyhow::Error::from)
                .and_then(|catalog| wait_for_writes(&writer, &catalog))
                .map_err(|e| format!("{e:#}"));
            let _ = tx.send(waited);
        });
        let waited = rx
            .recv_timeout(std::time::Duration::from_secs(60))
            .expect("gallery write kept waiting for a writer that ended");
        let error = waited.unwrap_err();
        assert!(error.contains("the file writer stopped with 1 changes still to write"), "{error}");
        quiet.check();
    }
}
