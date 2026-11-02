//! The command line of `gallery.exe` (`gallery scan`, `gallery import`, …): the app's passes without the window,
//! for scripts, profiling and checking results. Progress goes to stderr.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result};
use clap::{CommandFactory, Parser, Subcommand};
use pg_core::catalog::Catalog;
use pg_core::thumbs::ThumbCache;
use windows_sys::Win32::System::Threading::{THREAD_PRIORITY_IDLE, THREAD_PRIORITY_NORMAL};

use crate::control::Control;
use crate::writeback::Writer;
use crate::{borders, dups, import, pipeline, scan, thumbs};

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
}

impl PipelineArgs {
    fn options(&self, priority: i32) -> pipeline::Options {
        let mut opts = pipeline::Options::new(priority);
        opts.readers = self.readers.max(1);
        opts.workers = self.workers.unwrap_or(opts.workers).max(1);
        let budget = pipeline::Options::budget(opts.readers, opts.workers);
        opts.buffer_bytes = self.buffer_mb.map_or(budget, |mb| mb.max(1) << 20);
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
    },
    /// Thumbnails at the given grid height for every imported file (the app does this in the background).
    Thumbs {
        /// Grid row height in device pixels; any value, chosen at runtime.
        #[arg(long)]
        height: u32,
        /// Set the cache's size limit, kept in the cache: the app keeps to it too. Without it, the limit stays as it
        /// is (50 GB for a new cache).
        #[arg(long)]
        cap_gb: Option<f64>,
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
fn report() -> pg_core::thumbs::Report {
    Box::new(|e| eprintln!("{e:#}"))
}

/// Files left for the next pass because they changed while they were read, when there are any.
fn changed_while_read(n: usize) -> String {
    if n == 0 { String::new() } else { format!(", {n} changed while read, left for the next pass") }
}

/// Whether these arguments (without the program name) ask for the command line rather than the app.
pub fn wants_cli(args: &[OsString]) -> bool {
    let cmd = Cli::command();
    args.iter().any(|a| {
        let a = a.to_string_lossy();
        matches!(a.as_ref(), "help" | "--help" | "-h" | "--version" | "-V")
            || cmd.get_subcommands().any(|s| s.get_name() == a)
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
    let mut catalog = Catalog::open(&catalog_path)?;
    let ctl = Control::new(true);
    let start = Instant::now();

    match cli.command {
        Command::Scan { roots } => {
            let roots = if roots.is_empty() {
                catalog.roots()?
            } else {
                let mut normalized = Vec::new();
                for root in roots {
                    let root = scan::normalize_root(&root)?;
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
            let root = scan::normalize_root(&root).unwrap_or(root);
            let n = catalog.remove_root(&root)?;
            println!("{root}: removed from the library, {n} files hidden");
        }
        Command::Import { pipeline, retry_errors } => {
            let opts = pipeline.options(THREAD_PRIORITY_NORMAL);
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
        Command::Thumbs { height, cap_gb, pipeline, foreground } => {
            // Idle: only the cores the user leaves free. I/O priority stays normal: Windows' background mode
            // also throttles reads on an idle disk and made the task 6.5× slower (64 vs 415 files/s on dups2).
            let opts = pipeline.options(if foreground { THREAD_PRIORITY_NORMAL } else { THREAD_PRIORITY_IDLE });
            let cache = ThumbCache::open(
                &cli.catalog_dir,
                &catalog.catalog_id()?,
                cap_gb.map(|gb| (gb * 1e9) as i64),
                report(),
            )?;
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
                let s = borders::find(&mut catalog, &pipeline.options(THREAD_PRIORITY_NORMAL), &ctl)?;
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
                println!("library: {root}");
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
            let mut errors = catalog.conn.prepare("SELECT error FROM file_writes WHERE error IS NOT NULL LIMIT 5")?;
            for e in errors.query_map([], |r| r.get::<_, String>(0))? {
                println!("  {}", e?);
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
                let left: i64 = catalog.conn.query_row(
                    "SELECT count(*) FROM file_writes WHERE attempts < 5 AND (error IS NULL OR temp IS NOT NULL)",
                    [],
                    |r| r.get(0),
                )?;
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

/// `gallery borders --stdin`, or `gallery borders --csv <out> <paths>`.
fn borders_outside(paths: &[PathBuf], csv: Option<&Path>, stdin: bool, pipeline: &PipelineArgs) -> Result<()> {
    if stdin {
        let n = borders::serve(std::io::stdin().lock(), std::io::stdout().lock())?;
        eprintln!("borders looked for in {n} images");
        return Ok(());
    }
    let csv = csv.context("--csv: where the report goes")?;
    let s = borders::report(paths, csv, &pipeline.options(THREAD_PRIORITY_NORMAL), &Control::new(true))?;
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
