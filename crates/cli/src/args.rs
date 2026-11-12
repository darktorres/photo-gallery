//! The command line's arguments: the catalog folder, the commands, and the pipeline's knobs.

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use pg_jobs::pipeline;
use pg_model::size::ThumbHeight;
use pg_platform::priority::Priority;

use crate::dups;

#[derive(Parser)]
#[command(name = "gallery", version, about = "Photo gallery. Without a command, opens the app.")]
pub(crate) struct Cli {
    /// Folder holding catalog.sqlite and the thumbnail cache (default: the data folder's `catalog`, see `pg_library::layout`).
    // Named explicitly: the field is `catalog_dir_arg`, and the flag is `--catalog-dir` (README, the screenshot script).
    #[arg(long = "catalog-dir", env = "PG_CATALOG_DIR")]
    catalog_dir_arg: Option<PathBuf>,
    #[command(subcommand)]
    pub(crate) command: Command,
}

impl Cli {
    /// The data folders: the catalog folder the flag gives, else the environment's and the default (`pg_library::layout`).
    pub(crate) fn layout(&self) -> anyhow::Result<pg_library::layout::Layout> {
        pg_library::layout::Layout::resolve(|name| std::env::var_os(name), self.catalog_dir_arg.clone())
            .map_err(anyhow::Error::msg)
    }
}

#[derive(clap::Args)]
pub(crate) struct PipelineArgs {
    /// Reads in flight (IO.md "I/O": ≥ 8 reaches the HDD's sequential speed).
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
    pub(crate) fn options(&self, priority: Priority) -> pipeline::Options {
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
pub(crate) enum Command {
    /// Add folders to the library and record their image files (identity, size, mtime); files no longer there
    /// are marked missing. Without folders, rescans the whole library.
    Scan {
        roots: Vec<PathBuf>,
        /// When another process is scanning a folder that overlaps one of these (the app's job, another `gallery
        /// scan`), wait for it to end instead of failing.
        #[arg(long)]
        wait: bool,
    },
    /// Take a folder out of the library (its files are hidden, not deleted from the catalog).
    Remove { root: PathBuf },
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
        again: Vec<PathBuf>,
    },
    /// Thumbnails at the given grid height for every imported file (the app does this in the background).
    Thumbs {
        /// Grid row height in device pixels: the thumbnail cache's heights, 16 to 2048.
        #[arg(long, value_parser = thumb_height)]
        height: ThumbHeight,
        /// Set the cache's size limit in GB, kept in the cache: the app keeps to it too. Without it, the limit
        /// stays as it is (50 GB for a new cache).
        #[arg(long = "cap-gb", value_parser = cap_gb)]
        cap: Option<pg_thumbcache::Cap>,
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

/// `--cap-gb`: a number of GB above zero, as the thumbnail cache's cap.
fn thumb_height(text: &str) -> Result<ThumbHeight, String> {
    let px: u32 = text.trim().parse().map_err(|_| format!("{text} isn't a number of pixels"))?;
    ThumbHeight::new(px).ok_or_else(|| format!("{text} px isn't a height the thumbnail cache holds (16 to 2048)"))
}

fn cap_gb(text: &str) -> Result<pg_thumbcache::Cap, String> {
    let gb: f64 = text.trim().parse().map_err(|_| format!("{text} isn't a number of GB"))?;
    pg_thumbcache::Cap::from_gb(gb)
        .ok_or_else(|| format!("{text} GB isn't a size a cache can have (a number of GB above zero)"))
}
