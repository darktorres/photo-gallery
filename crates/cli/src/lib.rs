//! The command line of `gallery.exe` (`gallery scan`, `gallery import`, …): the app's passes without the window,
//! for scripts, profiling and checking results. Progress goes to stderr.

// The command line's output is its product: its reports go to stdout, its progress and errors to stderr.
#![expect(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "the command line's reports and progress are its output"
)]

mod borders;
mod dups;

use pg_model::ids::FileId;
use pg_model::rules::{Path as RulePath, PathRules};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use pg_catalog::Catalog;
use pg_platform::priority::Priority;
use pg_thumbcache::ThumbCache;

use pg_jobs::control::{Control, Event, Phase, Progress};
use pg_jobs::events::Bus;
use pg_jobs::pipeline;
use pg_writeback::Writer;

#[derive(Parser)]
#[command(name = "gallery", version, about = "Photo gallery. Without a command, opens the app.")]
struct Cli {
    /// Folder holding catalog.sqlite and the thumbnail cache (default: `%LOCALAPPDATA%\PhotoGallery\catalog`).
    // Named explicitly: the field is `catalog_dir_arg`, and the flag is `--catalog-dir` (README, the screenshot script).
    #[arg(long = "catalog-dir", env = "PG_CATALOG_DIR")]
    catalog_dir_arg: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

impl Cli {
    /// The catalog folder: the one given, else the default.
    fn catalog_dir(&self) -> PathBuf {
        self.catalog_dir_arg.clone().unwrap_or_else(pg_library::default_catalog_dir)
    }
}

#[derive(clap::Args)]
struct PipelineArgs {
    /// Reads in flight (FINDINGS "I/O": ≥ 8 reaches the HDD's sequential speed).
    #[arg(long, default_value_t = 8)]
    readers: usize,
    /// Decode threads (default: all logical CPUs).
    #[arg(long)]
    workers: Option<usize>,
    /// Megabytes of file buffers the pass holds at most (default: 12 per reader and per worker).
    #[arg(long)]
    buffer_mb: Option<usize>,
    /// Files in flight at most: being read, waiting, or being processed (default: one per reader and two per
    /// worker).
    #[arg(long)]
    slots: Option<usize>,
}

impl PipelineArgs {
    fn options(&self, priority: Priority) -> pipeline::Options {
        let mut opts = pipeline::Options::new(priority);
        opts.readers = self.readers.max(1);
        opts.workers = self.workers.unwrap_or(opts.workers).max(1);
        let budget = pipeline::Options::budget(opts.readers, opts.workers);
        opts.buffer_bytes = self.buffer_mb.map_or(budget, |mb| mb.max(1) << 20);
        opts.slots = self.slots.unwrap_or(pipeline::Options::slots(opts.readers, opts.workers)).max(1);
        opts
    }
}

#[derive(Subcommand)]
enum Command {
    /// Add folders to the library and record their image files (identity, size, mtime); files no longer there
    /// are marked missing. Without folders, rescans the whole library.
    Scan { roots: Vec<String> },
    /// Take a folder out of the library (its files are hidden, not deleted from the catalog).
    Remove { root: String },
    /// Analyze every new or changed file: hashes, PDQ (+ tier-1 pairs), dimensions, ThumbHash.
    Import {
        #[command(flatten)]
        pipeline: PipelineArgs,
        /// Also retry files that failed before.
        #[arg(long)]
        retry_errors: bool,
        /// Analyze these files, and the files under these folders, again, whatever their analysis: after a fix to
        /// what the import reads, which does not bump IMPORT_VERSION.
        #[arg(long, num_args = 1.., value_name = "PATH")]
        again: Vec<String>,
    },
    /// Thumbnails at the given grid height for every imported file (the app does this in the background).
    Thumbs {
        /// Grid row height in device pixels; any value, chosen at runtime.
        #[arg(long)]
        height: u32,
        /// Set the cache's size limit in GB, kept in the cache: the app keeps to it too. Without it, the limit
        /// stays as it is (50 GB for a new cache).
        #[arg(long = "cap-gb", value_parser = pg_thumbcache::parse_cap_gb)]
        cap: Option<i64>,
        #[command(flatten)]
        pipeline: PipelineArgs,
        /// Run at normal CPU priority instead of idle.
        #[arg(long)]
        foreground: bool,
    },
    /// Borders around images. Without paths: look for those due, in what the import analyzed and after a new
    /// detector (the app does this in the background, after the import).
    Borders {
        /// Report on these files or folders (searched recursively) instead, into --csv, in the Python prototype's
        /// report.csv columns (the prototype, tools/borders.py, is not in this repo). The catalog isn't touched.
        paths: Vec<PathBuf>,
        /// Where the report goes.
        #[arg(long, requires = "paths")]
        csv: Option<PathBuf>,
        /// Mark the files a review.json of the eval viewer's former Borders page calls false positives as not a
        /// border.
        #[arg(long, conflicts_with = "paths")]
        import_review: Option<PathBuf>,
        /// Look for borders in images piped in, for the eval (`borders::serve`): each one's width and height (u32,
        /// little-endian) then its RGB pixels; for each, a line with what is left (left top right bottom). The
        /// catalog isn't touched.
        #[arg(long, conflicts_with_all = ["paths", "import_review"])]
        stdin: bool,
        #[command(flatten)]
        pipeline: PipelineArgs,
    },
    /// Write the ratings and turns made in the app into the files, until none is left (the app does this in the
    /// background).
    Write,
    /// The library folders and counts of files, analyses, pairs and pending writes.
    Status,
    /// What tiers 0–1 found, and the groups they form.
    Dups {
        /// PDQ distance that joins two files into a group.
        #[arg(long, default_value_t = dups::DEFAULT_GROUP_DISTANCE)]
        distance: u32,
        /// Print the files of the N largest groups.
        #[arg(long, default_value_t = 0)]
        show: usize,
    },
}

/// The thumbnail cache's events, on stderr with the progress as they come. A thread prints them, and ends once the
/// cache is gone (its bus with it): joining the thread then means the last one is out.
fn thumb_events() -> (Arc<Bus<pg_thumbcache::Event>>, JoinHandle<()>) {
    let bus = Arc::new(Bus::new(64));
    let events = bus.subscribe();
    let printer = std::thread::spawn(move || {
        while let Some(event) = events.recv() {
            eprintln!("{}", thumb_event_text(&event));
        }
    });
    (bus, printer)
}

/// What a thumbnail cache event says, on the command line.
fn thumb_event_text(event: &pg_thumbcache::Event) -> String {
    use pg_thumbcache::Event;
    match event {
        Event::TotalsFixed { kept, actual } => {
            format!("thumbnail cache: totals were {kept:?}, the entries hold {actual:?}; fixed")
        }
        Event::TouchesNotWritten { error } => format!("thumbnail cache: touches not written yet: {error}"),
        Event::NotStored { error } => format!("thumbnail cache: a thumbnail not stored: {error}"),
        Event::WalNotMerged { error } => format!("thumbnail cache: the WAL was not merged at close: {error}"),
        Event::StartedOver { why } => format!("thumbnail cache: started over, {why}"),
    }
}

/// Says on stderr that the catalog was rebuilt, why, where the old file went, and what was carried over into the new one
/// (or why nothing was, which says to scan the library again).
fn report_rebuilt(catalog: &Catalog) {
    if let Some(r) = catalog.rebuilt() {
        eprintln!("catalog rebuilt: {}; the old one is kept as {}{}", r.why, r.kept.display(), r.carried_note());
    }
}

/// Files left for the next pass because they changed while they were read, when there are any.
fn changed_while_read(n: usize) -> String {
    if n == 0 { String::new() } else { format!(", {n} changed while read, left for the next pass") }
}

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
    std::fs::create_dir_all(cli.catalog_dir())?;
    with_progress(|ctl| run_command(cli, ctl))
}

/// Runs `f` with a control whose progress is printed on stderr while it runs: a line every few seconds for each
/// phase, and the phase's last line when it ends. The printer is joined before this returns.
fn with_progress<T>(f: impl FnOnce(&Control) -> T) -> T {
    let ctl = Control::new();
    let events = ctl.subscribe();
    let printer = std::thread::spawn(move || {
        while let Some(event) = events.recv() {
            match event {
                Event::Progress(p) | Event::Finished(p) => eprintln!("{}", progress_line(&p)),
            }
        }
    });
    let out = f(&ctl);
    drop(ctl);
    let _ = printer.join();
    out
}

/// The command line's progress line for a phase.
fn progress_line(p: &Progress) -> String {
    let secs = p.elapsed().as_secs_f64().max(1e-3);
    let label = phase_label(&p.phase);
    if p.total == 0 {
        return format!("  {label}: {} files, {:.0} files/s, {secs:.0} s", p.done, p.rate());
    }
    format!(
        "  {label}: {}/{} ({:.1}%), {:.0} files/s, {:.0} MB/s, {} failed, {secs:.0} s elapsed, ETA {:.0} s",
        p.done,
        p.total,
        100.0 * p.fraction(),
        p.rate(),
        pg_model::num::size(p.bytes) / secs / 1e6,
        p.failed,
        p.eta()
    )
}

fn phase_label(phase: &Phase) -> String {
    match phase {
        Phase::Idle => "idle".into(),
        Phase::Crop => "crop".into(),
        Phase::Scan { root } => format!("scan {root}"),
        Phase::Import => "import".into(),
        Phase::Borders => "borders".into(),
        Phase::Thumbs { height } => format!("thumbs {height} px"),
    }
}

/// `text` as a path under the catalog's rules; text that is none is an error.
fn rule_path(rules: PathRules, text: &str) -> Result<RulePath> {
    rules.parse(text).ok_or_else(|| anyhow::anyhow!("{text} is not a path"))
}

fn run_command(cli: Cli, ctl: &Control) -> Result<()> {
    // Before the match below moves out of `cli.command`: the catalog folder is read from the whole `cli`.
    let catalog_dir = cli.catalog_dir();
    let catalog_path = catalog_dir.join("catalog.sqlite");
    let mut catalog = Catalog::prepare(&catalog_path, pg_library::versions(), PathRules::NATIVE)?;
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
                    let root = pg_scan::normalize_path(Path::new(&root))?;
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
                    let cache = ThumbCache::open(&catalog_dir, &catalog.catalog_id()?, None, events)?;
                    for id in &s.replaced_ids {
                        if let Err(e) = cache.forget(*id) {
                            println!("{name}: a replaced file's thumbnail wasn't deleted: {e:#}");
                        }
                    }
                    drop(cache);
                    let _ = printer.join();
                }
                for temp in &s.temps {
                    let disk = pg_platform::names::path_buf(&rules.join(temp));
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
            let root = match pg_scan::normalize_path(Path::new(&root)) {
                Ok(path) => path,
                Err(_) => rule_path(rules, &root)?,
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
            let cache = ThumbCache::open(&catalog_dir, &catalog.catalog_id()?, cap, events)?;
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
                let s = pg_library::borders::find(&mut catalog, &pipeline.options(Priority::Normal), ctl)?;
                println!(
                    "borders looked for in {} files ({} failed), {:.1} s",
                    s.files,
                    s.failed,
                    start.elapsed().as_secs_f64()
                );
            }
        }
        Command::Status => {
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
                Box::leak(Box::new(ThumbCache::open(&catalog_dir, &catalog.catalog_id()?, None, events)?));
            let writer = Writer::start(catalog_path.clone(), cache, Arc::new(Bus::new(16)))?;
            writer.wake();
            // Until nothing writable is left (failures past their retries stay queued and are reported).
            loop {
                std::thread::sleep(std::time::Duration::from_millis(500));
                let left = pg_catalog::writes::left_to_write(&catalog)?;
                if left == 0 {
                    break;
                }
            }
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

/// The present files of the catalog at these paths, each a file or a folder (its files and its subfolders'), each
/// once. Fails, before anything is done, on a path that holds none.
fn files_named(catalog: &Catalog, paths: &[String]) -> Result<Vec<FileId>> {
    let mut ids = Vec::new();
    for path in paths {
        let rule = pg_scan::normalize_path(Path::new(path))?;
        let before = ids.len();
        if Path::new(path).is_dir() {
            ids.extend(catalog.files_under(&rule)?);
        } else {
            ids.extend(catalog.file_by_path(&rule)?);
        }
        if ids.len() == before {
            bail!("{path}: no file of the library there");
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

/// The first failures a pass kept, one per line on stderr, under `what`.
fn print_failures(what: &str, failures: &[pg_jobs::failure::Failure]) {
    for f in failures {
        eprintln!("{what}: {}: {}", f.path, f.error);
    }
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
        assert_eq!(cli.catalog_dir(), PathBuf::from("X"));
    }
}
