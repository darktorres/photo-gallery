//! `pg-driver`: the developer's command line over `pg_driver::Headless`, for scripts and measurements. Not shipped. Every command takes
//! `--home`, the folder its catalog, preferences and log live in; nothing is read from the user's library. Results are printed as
//! JSON, one object per line, on stdout; progress and errors go to stderr.

#![forbid(unsafe_code)]

use std::fmt::Display;
use std::io::{self, Write};
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use pg_driver::{Headless, Setup};
use pg_library::mask::{PassKind, PassMask};
use pg_model::size::ThumbHeight;
use serde_json::{Value, json};

#[derive(Parser)]
#[command(name = "pg-driver", about = "The developer's headless driver of the photo gallery's library. Not shipped.")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Adds folders, starts the library, and runs its passes until it is quiet: once for each height (or once without one).
    Run {
        /// The home folder (the catalog, the preferences and the log live in it).
        #[arg(long)]
        home: PathBuf,
        /// Makes the home's catalog folder when it is missing. Without it a missing home is refused.
        #[arg(long)]
        create: bool,
        /// A folder to add to the library before the run.
        #[arg(long = "add", value_name = "PATH")]
        add: Vec<PathBuf>,
        /// A thumbnail height in pixels to make thumbnails at, once per height in order (16 to 2048).
        #[arg(long = "height", value_name = "PX")]
        heights: Vec<u32>,
        /// The passes to run, comma-separated: scans, import, thumbs, placeholders, hashes, pairing, borders. All by default.
        #[arg(long, value_delimiter = ',', value_name = "NAMES")]
        passes: Vec<String>,
        /// The reads in flight, pinned for the run (a sweep). A pinned run writes no learned profile.
        #[arg(long, value_name = "N")]
        pin_readers: Option<NonZeroUsize>,
        /// The decode threads, pinned for the run.
        #[arg(long, value_name = "N")]
        pin_workers: Option<NonZeroUsize>,
        /// Seconds to wait for the library to be quiet, for each run.
        #[arg(long, default_value_t = 600, value_name = "SECONDS")]
        timeout: u64,
        /// The perf log to append to (`PG_PERF_LOG` gives one too); its table is printed when the run ends.
        #[arg(long, value_name = "FILE")]
        perf: Option<PathBuf>,
    },
    /// The catalog's counts, folders and unfinished removals.
    Status {
        /// The home folder.
        #[arg(long)]
        home: PathBuf,
        /// Makes the home's catalog folder when it is missing. Without it a missing home is refused.
        #[arg(long)]
        create: bool,
    },
    /// Removes a library folder, and finishes its removal (or gives up its pending writes with `--discard-pending`).
    Remove {
        /// The home folder.
        #[arg(long)]
        home: PathBuf,
        /// Makes the home's catalog folder when it is missing. Without it a missing home is refused.
        #[arg(long)]
        create: bool,
        /// The library folder, as the catalog has it.
        path: String,
        /// Gives up the removal that waits for its unfinished writes, deleting their temporary files.
        #[arg(long)]
        discard_pending: bool,
        /// Seconds to wait for the removal to finish.
        #[arg(long, default_value_t = 600, value_name = "SECONDS")]
        timeout: u64,
    },
    /// Writes the queued changes (ratings and turns) into the files, until none is left.
    Write {
        /// The home folder.
        #[arg(long)]
        home: PathBuf,
        /// Makes the home's catalog folder when it is missing. Without it a missing home is refused.
        #[arg(long)]
        create: bool,
        /// Seconds to wait for the writes to finish.
        #[arg(long, default_value_t = 600, value_name = "SECONDS")]
        timeout: u64,
    },
    /// The table of a perf log: each step's time, its share, what it processed, and where each pipeline was bound.
    Perf {
        /// The perf log file.
        file: PathBuf,
    },
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            // The exit status is the failure whether or not the message reaches the console.
            let _ = stderr_line(format!("pg-driver: {error:#}"));
            ExitCode::FAILURE
        }
    }
}

/// Writes `text` and a newline to stdout, as one line: a result.
fn stdout_line(text: impl Display) -> io::Result<()> {
    let mut out = io::stdout().lock();
    writeln!(out, "{text}")
}

/// Writes `text` and a newline to stderr, as one line: progress and messages.
fn stderr_line(text: impl Display) -> io::Result<()> {
    let mut err = io::stderr().lock();
    writeln!(err, "{text}")
}

/// The setup of a home, with whether a missing home is made.
fn setup(home: PathBuf, create: bool) -> Setup {
    Setup { create, ..Setup::new(home) }
}

fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Command::Run { home, create, add, heights, passes, pin_readers, pin_workers, timeout, perf } => {
            let mask = pass_mask(&passes)?;
            let timeout = Duration::from_secs(timeout);
            let mut setup = setup(home, create);
            setup.pin_readers = pin_readers;
            setup.pin_workers = pin_workers;
            let perf = perf.or_else(|| pg_platform::env::read(pg_platform::env::Var::PerfLog).map(PathBuf::from));
            with_perf(perf.as_deref(), || {
                let headless = Headless::open(&setup)?;
                headless.start()?;
                for folder in &add {
                    let (_, added) = headless.add_folder(folder)?;
                    stderr_line(format!("added {} (new: {added})", folder.display()))?;
                }
                let heights: Vec<Option<u32>> =
                    if heights.is_empty() { vec![None] } else { heights.into_iter().map(Some).collect() };
                for height in heights {
                    if let Some(px) = height {
                        let height = ThumbHeight::new(px)
                            .with_context(|| format!("{px} px is not a thumbnail height (16 to 2048)"))?;
                        headless.set_thumb_height(height);
                    }
                    let started = Instant::now();
                    let elapsed = headless.run(mask, timeout)?;
                    print_json(&json!({
                        "command": "run",
                        "height": height,
                        "passes": pass_names(mask),
                        "seconds": elapsed.as_secs_f64(),
                        "wall_seconds": started.elapsed().as_secs_f64(),
                        "status": status_json(&headless.status()?),
                    }))?;
                }
                Ok(())
            })
        }
        Command::Status { home, create } => {
            let headless = Headless::open(&setup(home, create))?;
            print_json(&json!({ "command": "status", "status": status_json(&headless.status()?) }))
        }
        Command::Remove { home, create, path, discard_pending, timeout } => {
            let headless = Headless::open(&setup(home, create))?;
            let timeout = Duration::from_secs(timeout);
            if discard_pending {
                let discarded = headless.discard_removal(&path)?;
                print_json(&json!({ "command": "remove", "path": path, "discarded": discarded }))
            } else {
                let removed = headless.remove_folder(&path, timeout)?;
                print_json(&json!({ "command": "remove", "path": path, "removed": removed }))
            }
        }
        Command::Write { home, create, timeout } => {
            let mut setup = setup(home, create);
            setup.writer = true;
            let headless = Headless::open(&setup)?;
            let queued = headless.write_pending(Duration::from_secs(timeout))?;
            print_json(&json!({ "command": "write", "queued": queued, "status": status_json(&headless.status()?) }))
        }
        Command::Perf { file } => {
            let table = pg_perf::summarize(&file).with_context(|| format!("reading {}", file.display()))?;
            for line in table.lines() {
                stdout_line(line)?;
            }
            Ok(())
        }
    }
}

/// Runs `body` with the perf log at `path` open (when there is one), and prints its table when the run ends.
fn with_perf(path: Option<&std::path::Path>, body: impl FnOnce() -> Result<()>) -> Result<()> {
    let Some(path) = path else { return body() };
    let meta = json!({ "program": pg_perf::PROGRAM, "version": env!("CARGO_PKG_VERSION"), "command": "pg-driver run" });
    pg_perf::open(path, meta).with_context(|| format!("opening the perf log {}", path.display()))?;
    let outcome = {
        let _root = pg_perf::span("cmd.driver.run");
        body()
    };
    let table = pg_perf::summarize_session(path, pg_perf::session_id())
        .with_context(|| format!("reading the perf log {}", path.display()))?;
    for line in table.lines() {
        stderr_line(line)?;
    }
    outcome
}

/// The pass set named on the command line: all passes when none is named.
fn pass_mask(names: &[String]) -> Result<PassMask> {
    if names.is_empty() {
        return Ok(PassMask::ALL);
    }
    let mut kinds = Vec::with_capacity(names.len());
    for name in names {
        kinds.push(match name.trim() {
            "scans" => PassKind::Scans,
            "import" => PassKind::Import,
            "thumbs" => PassKind::Thumbs,
            "placeholders" => PassKind::Placeholders,
            "hashes" => PassKind::Hashes,
            "pairing" => PassKind::Pairing,
            "borders" => PassKind::Borders,
            other => bail!("{other} is not a pass: scans, import, thumbs, placeholders, hashes, pairing or borders"),
        });
    }
    Ok(PassMask::only(&kinds))
}

/// The names of the passes a mask runs, in round order.
fn pass_names(mask: PassMask) -> Vec<&'static str> {
    PassKind::ALL
        .iter()
        .filter(|kind| mask.contains(**kind))
        .map(|kind| match kind {
            PassKind::Scans => "scans",
            PassKind::Import => "import",
            PassKind::Thumbs => "thumbs",
            PassKind::Placeholders => "placeholders",
            PassKind::Hashes => "hashes",
            PassKind::Pairing => "pairing",
            PassKind::Borders => "borders",
        })
        .collect()
}

/// A catalog status as JSON.
fn status_json(status: &pg_driver::Status) -> Value {
    let counts = &status.counts;
    json!({
        "files": counts.files,
        "missing": counts.missing,
        "analyzed": counts.analyzed,
        "failed": counts.failed,
        "pending": counts.pending,
        "pdq_pairs": counts.pdq_pairs,
        "queued_writes": counts.queued_writes,
        "failed_writes": counts.failed_writes,
        "roots": status.roots,
        "removals": status.removals,
    })
}

/// One JSON object, on one line of stdout.
fn print_json(value: &Value) -> Result<()> {
    stdout_line(serde_json::to_string(value).context("encoding the result")?)?;
    Ok(())
}
