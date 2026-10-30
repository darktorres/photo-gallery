//! The command line of `gallery.exe` (`gallery scan`, `gallery import`, …): the app's passes without the window,
//! for scripts, profiling and checking results. Progress goes to stderr.

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Instant;

use anyhow::Result;
use clap::{CommandFactory, Parser, Subcommand};
use pg_core::catalog::Catalog;
use pg_core::thumbs::ThumbCache;
use windows_sys::Win32::System::Threading::{THREAD_PRIORITY_IDLE, THREAD_PRIORITY_NORMAL};

use crate::control::Control;
use crate::{dups, import, pipeline, scan, thumbs};

pub const DEFAULT_CATALOG_DIR: &str = r"D:\PhotoGallery\catalog";

#[derive(Parser)]
#[command(name = "gallery", version, about = "Photo gallery. Without a command, opens the app.")]
struct Cli {
    /// Folder holding catalog.sqlite and thumbs.sqlite (on D:, never C:).
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
}

impl PipelineArgs {
    fn options(&self, priority: i32) -> pipeline::Options {
        let mut opts = pipeline::Options::new(priority);
        opts.readers = self.readers.max(1);
        opts.workers = self.workers.unwrap_or(opts.workers).max(1);
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
        /// Cache size limit.
        #[arg(long, default_value_t = 50.0)]
        cap_gb: f64,
        #[command(flatten)]
        pipeline: PipelineArgs,
        /// Run at normal CPU priority instead of idle.
        #[arg(long)]
        foreground: bool,
    },
    /// The library folders and counts of files, analyses and pairs.
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
                    "{root}: {} images in {} folders, {} now missing, {} unreadable folders ({:.1} s)",
                    s.files,
                    s.folders,
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
                "imported {} files ({} failed), {} PDQ pairs, {:.1} s",
                s.files,
                s.failed,
                s.pairs,
                start.elapsed().as_secs_f64()
            );
        }
        Command::Thumbs { height, cap_gb, pipeline, foreground } => {
            // Idle: only the cores the user leaves free. I/O priority stays normal: Windows' background mode
            // also throttles reads on an idle disk and made the task 6.5× slower (64 vs 415 files/s on dups2).
            let opts = pipeline.options(if foreground { THREAD_PRIORITY_NORMAL } else { THREAD_PRIORITY_IDLE });
            let cache = Mutex::new(ThumbCache::open(&cli.catalog_dir.join("thumbs.sqlite"), (cap_gb * 1e9) as i64)?);
            let s = thumbs::run(&cache, &catalog_path, height, &opts, &ctl)?;
            println!(
                "{} thumbnails at {height} px ({} failed){}; cache {:.2} GB, {:.1} s",
                s.made,
                s.failed,
                if s.cache_full { ", stopped: cache full" } else { "" },
                cache.lock().unwrap().total_bytes() as f64 / 1e9,
                start.elapsed().as_secs_f64()
            );
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
        }
        Command::Dups { distance, show } => dups::report(&catalog, distance, show)?,
    }
    Ok(())
}
