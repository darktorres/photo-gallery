//! `pg`: scan folders into the catalog, import (analyze) them, make thumbnails in the background, and report
//! tiers 0–1. See PLAN §8 M1.

mod dups;
mod import;
mod pipeline;
mod progress;
mod scan;
mod thumbs;

use std::path::PathBuf;
use std::time::Instant;

use anyhow::Result;
use clap::{Parser, Subcommand};
use pg_core::catalog::Catalog;
use pg_core::thumbs::ThumbCache;

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    /// Folder holding catalog.sqlite and thumbs.sqlite (on D:, never C:).
    #[arg(long, env = "PG_CATALOG_DIR", default_value = r"D:\PhotoGallery\catalog")]
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
    fn options(&self) -> pipeline::Options {
        let cpus = std::thread::available_parallelism().map_or(4, |n| n.get());
        pipeline::Options { readers: self.readers.max(1), workers: self.workers.unwrap_or(cpus).max(1) }
    }
}

#[derive(Subcommand)]
enum Command {
    /// Record the image files under each folder (identity, size, mtime); unseen files are marked missing.
    Scan { roots: Vec<String> },
    /// Analyze every new or changed file: hashes, PDQ (+ tier-1 pairs), dimensions, ThumbHash.
    Import {
        #[command(flatten)]
        pipeline: PipelineArgs,
        /// Also retry files that failed before.
        #[arg(long)]
        retry_errors: bool,
    },
    /// Background task: thumbnails at the given grid height for every imported file.
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
    /// Counts of files, analyses and pairs.
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

fn main() -> Result<()> {
    let cli = Cli::parse();
    std::fs::create_dir_all(&cli.catalog_dir)?;
    let catalog_path = cli.catalog_dir.join("catalog.sqlite");
    let mut catalog = Catalog::open(&catalog_path)?;
    let start = Instant::now();

    match cli.command {
        Command::Scan { roots } => {
            for root in roots {
                let s = scan::scan(&mut catalog, &root)?;
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
        Command::Import { pipeline, retry_errors } => {
            let s = import::import(&mut catalog, &pipeline.options(), retry_errors)?;
            println!(
                "imported {} files ({} failed), {} PDQ pairs, {:.1} s",
                s.files,
                s.failed,
                s.pairs,
                start.elapsed().as_secs_f64()
            );
        }
        Command::Thumbs { height, cap_gb, pipeline, foreground } => {
            if !foreground {
                background_priority();
            }
            let mut cache = ThumbCache::open(&cli.catalog_dir.join("thumbs.sqlite"), (cap_gb * 1e9) as i64)?;
            let s = thumbs::run(&mut cache, &catalog_path, height, &pipeline.options())?;
            println!(
                "{} thumbnails at {height} px ({} failed){}; cache {:.2} GB, {:.1} s",
                s.made,
                s.failed,
                if s.cache_full { ", stopped: cache full" } else { "" },
                cache.total_bytes() as f64 / 1e9,
                start.elapsed().as_secs_f64()
            );
        }
        Command::Status => {
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

/// Idle CPU priority, so the task only uses cores the user leaves free. I/O priority stays normal: Windows'
/// background mode (`PROCESS_MODE_BACKGROUND_BEGIN`) also throttles reads on an idle disk and made the task 6.5×
/// slower (64 vs 415 files/s on the dups2 smoke set).
fn background_priority() {
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, IDLE_PRIORITY_CLASS, SetPriorityClass};
    unsafe { SetPriorityClass(GetCurrentProcess(), IDLE_PRIORITY_CLASS) };
}
