//! The command line of `gallery.exe` (`gallery scan`, `gallery import`, …): the app's passes without the window,
//! for scripts, profiling and checking results. Progress goes to stderr.

use pg_model::ids::FileId;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use pg_catalog::Catalog;
use pg_platform::priority::Priority;
use pg_thumbcache::ThumbCache;

use crate::writeback::Writer;
use crate::{borders, dups, import, scan, thumbs};
use pg_jobs::control::Control;
use pg_jobs::pipeline;

pub const DEFAULT_CATALOG_DIR: &str = r"D:\PhotoGallery\catalog";

#[derive(Parser)]
#[command(name = "gallery", version, about = "Photo gallery. Without a command, opens the app.")]
struct Cli {
    /// Folder holding catalog.sqlite and the thumbnail cache (on D:, never C:).
    #[arg(long, env = "PG_CATALOG_DIR", default_value = DEFAULT_CATALOG_DIR)]
    catalog_dir: PathBuf,
    #[command(subcommand)]
    command: Command,
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
        /// what the import reads, which comes without a new IMPORT_VERSION before the first release.
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
        /// report.csv columns (tools/borders.py, in git history). The catalog isn't touched.
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

/// The thumbnail cache's background failures, on stderr with the progress.
fn report() -> pg_thumbcache::Report {
    Box::new(|e| eprintln!("{e:#}"))
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
    std::fs::create_dir_all(&cli.catalog_dir)?;
    let catalog_path = cli.catalog_dir.join("catalog.sqlite");
    let mut catalog = Catalog::prepare(&catalog_path, crate::versions())?;
    let ctl = Control::new(true);
    let start = Instant::now();

    match cli.command {
        Command::Scan { roots } => {
            let roots = if roots.is_empty() {
                catalog.roots()?.into_iter().map(|r| r.path).collect()
            } else {
                let mut normalized = Vec::new();
                for root in roots {
                    let root = scan::normalize_path(&root)?;
                    if !catalog.add_root(&root)? {
                        println!("{root} is already in the library");
                    }
                    normalized.push(root);
                }
                normalized
            };
            for root in roots {
                let s = scan::scan(&mut catalog, &root, &ctl)?;
                println!(
                    "{root}: {} files in {} folders, {} new or changed, {} now missing, {} unreadable folders ({:.1} s)",
                    s.files,
                    s.folders,
                    s.changed,
                    s.missing,
                    s.errors,
                    start.elapsed().as_secs_f64()
                );
            }
        }
        Command::Remove { root } => {
            // A folder deleted from disk can't be normalized, and it is the one to remove: its path as given is then
            // the catalog's, and the catalog says whether it is a library folder.
            let root = scan::normalize_path(&root).unwrap_or(root);
            let removed = match catalog.root_id(&root)? {
                Some(id) => catalog.remove_root(id)?,
                None => None,
            };
            match removed {
                Some(n) => println!("{root}: removed from the library, {n} files hidden"),
                None => anyhow::bail!("{root}: not in the library"),
            }
        }
        Command::Import { pipeline, retry_errors, again } => {
            if !again.is_empty() {
                let marked = catalog.mark_pending(&files_named(&catalog, &again)?)?;
                println!("{marked} files to analyze again");
            }
            let opts = pipeline.options(Priority::Normal);
            let s = import::import(&mut catalog, &opts, retry_errors, &ctl)?;
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
            let cache = ThumbCache::open(&cli.catalog_dir, &catalog.catalog_id()?, cap, report())?;
            let s = thumbs::run(&cache, &catalog_path, height, &opts, &ctl)?;
            cache.flush();
            println!(
                "{} thumbnails at {height} px ({} failed{}){}; cache {:.2} of {:.2} GB, {:.1} s",
                s.made,
                s.failed,
                changed_while_read(s.changed),
                if s.cache_full { ", stopped: cache full" } else { "" },
                cache.totals()?.0 as f64 / 1e9,
                cache.cap()? as f64 / 1e9,
                start.elapsed().as_secs_f64()
            );
        }
        Command::Borders { import_review, pipeline, .. } => {
            if let Some(review) = import_review {
                let (marked, unknown) = borders::import_review(&catalog, &review)?;
                println!("{marked} files marked as not a border; {unknown} not in the catalog");
            } else {
                let s = borders::find(&mut catalog, &pipeline.options(Priority::Normal), &ctl)?;
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
                println!("library: {}", root.path);
            }
            let s = catalog.status()?;
            println!(
                "{} files ({} missing), {} analyzed, {} failed, {} pending, {} PDQ pairs",
                s.files, s.missing, s.analyzed, s.failed, s.pending, s.pdq_pairs
            );
            println!("{} changes to write into files ({} failing)", s.queued_writes, s.failed_writes);
            match ThumbCache::read_usage(&cli.catalog_dir)? {
                Some(u) => println!(
                    "thumbnail cache: {} thumbnails, {:.2} of {:.2} GB",
                    u.count,
                    u.bytes as f64 / 1e9,
                    u.cap as f64 / 1e9
                ),
                None => println!("thumbnail cache: none yet"),
            }
            for e in pg_catalog::writes::recent_errors(&catalog, 5)? {
                println!("  {e}");
            }
        }
        Command::Write => {
            // Only retagged here (never filled): the cap stays as it is.
            let cache: &'static ThumbCache =
                Box::leak(Box::new(ThumbCache::open(&cli.catalog_dir, &catalog.catalog_id()?, None, report())?));
            let writer = Writer::start(catalog_path.clone(), cache, || {});
            writer.wake();
            // Until nothing writable is left (failures past their retries stay queued and are reported).
            loop {
                std::thread::sleep(std::time::Duration::from_millis(500));
                let left = pg_catalog::writes::left_to_write(&catalog)?;
                if left == 0 {
                    break;
                }
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
        let real = scan::normalize_path(path)?;
        let before = ids.len();
        if Path::new(&real).is_dir() {
            ids.extend(catalog.files_under(&real)?);
        } else {
            ids.extend(catalog.file_by_path(&real)?);
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
    let s = borders::report(paths, csv, &pipeline.options(Priority::Normal), &Control::new(true))?;
    println!(
        "{} files: {} with borders, {} rejected candidates only, {} skipped, {} errors; {:.1} s ({:.0} files/s)",
        s.files,
        s.borders,
        s.rejected,
        s.skipped,
        s.errors,
        s.seconds,
        s.files as f64 / s.seconds.max(1e-3)
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
}
