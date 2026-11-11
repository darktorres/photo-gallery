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

use pg_client::text::{self, Message};
use pg_client::text::cli::{Line, line};
use pg_library::folders::AddError;
use pg_model::ids::FileId;
use pg_model::notice::{FilePass, Notice};
use pg_model::rules::{Path as RulePath, PathRules};
use pg_platform::names::{NormalizeError, shown};
use pg_scan::ScanStats;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result, anyhow, bail};
use clap::{Parser, Subcommand};
use pg_catalog::Catalog;
use pg_platform::priority::Priority;
use pg_thumbcache::ThumbCache;

use pg_jobs::control::Control;
use pg_jobs::events::Bus;
use pg_writeback::Writer;

use args::{Cli, Command, PipelineArgs};
use report::{Kept, changed_while_read, print_failures, report_pass, report_rebuilt, thumb_events, with_progress};

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

/// A path given on the command line, as the catalog has it ([`pg_platform::names::normalize_path`]); one that can't
/// be opened, or isn't one, is an error in the client's words.
fn normalized(path: &Path) -> Result<RulePath> {
    pg_platform::names::normalize_path(path).map_err(|e| anyhow!(path_words(&e)))
}

/// Why a path couldn't be taken, in the client's words.
fn path_words(e: &NormalizeError) -> String {
    match e {
        NormalizeError::Unreadable { path, error } => {
            text::render(&Message::PathUnreadable { path: shown(path), error: error.to_string() })
        }
        NormalizeError::NotAPath { path } => text::render(&Message::NotAPath { path: shown(path) }),
    }
}

/// Print a line of the command line's own.
fn say(l: &Line) {
    println!("{}", line(l));
}

/// Seconds as the command line shows them: one decimal.
fn secs(seconds: f64) -> String {
    format!("{seconds:.1}")
}

/// Gigabytes as the command line shows them, from bytes: two decimals.
fn gigabytes(bytes: f64) -> String {
    format!("{:.2}", bytes / 1e9)
}

/// What `gallery scan` says of a root it scanned in `seconds`: its counts, then the notices its result carries
/// (`pg_library::scans::scan_notices`: unreadable folders, refused mounts, the notes), as the window's inbox words them.
fn scan_report(root: &RulePath, s: &ScanStats, rules: PathRules, seconds: f64) -> Vec<String> {
    let summary = line(&Line::ScanSummary {
        root: rules.display(root),
        files: s.files.to_string(),
        folders: s.folders.to_string(),
        changed: s.changed.to_string(),
        missing: s.missing.to_string(),
        unreadable: s.errors.to_string(),
        seconds: secs(seconds),
    });
    let notices = pg_library::scans::scan_notices(root, s);
    std::iter::once(summary).chain(notices.iter().map(|n| text::notice_line(n, rules))).collect()
}

fn run_command(cli: Cli, ctl: &Control) -> Result<()> {
    // Before the match below moves out of `cli.command`: the data folders are read from the whole `cli`.
    // The catalog folder as SQLite opens it (an alias in the home for a folder it can't name).
    let layout = cli.layout()?.for_sqlite()?;
    let catalog_path = layout.catalog_path();
    let mut catalog = pg_library::open_catalog(&layout)?;
    report_rebuilt(&catalog);
    let rules = catalog.rules();
    let start = Instant::now();

    match cli.command {
        Command::Scan { roots } => {
            let roots: Vec<RulePath> = if roots.is_empty() {
                catalog.roots()?.into_iter().map(|r| r.path).collect()
            } else {
                let mut added = Vec::new();
                for root in roots {
                    match pg_library::folders::add_folder(&catalog, &root) {
                        Ok((root, new)) => {
                            if !new {
                                let notice = Notice::FolderAlreadyAdded { folder: root.clone() };
                                println!("{}", text::render(&Message::NoticeTitle { notice, rules }));
                            }
                            added.push(root);
                        }
                        // A refused folder is not added; the other folders still are.
                        Err(AddError::Refused { folder, reason }) => {
                            println!("{}", text::notice_line(&Notice::FolderRefused { folder, reason }, rules));
                        }
                        Err(AddError::Path(e)) => bail!(path_words(&e)),
                        Err(AddError::Catalog(e)) => return Err(e.into()),
                    }
                }
                added
            };
            for root in roots {
                let name = rules.display(&root);
                let s = match pg_scan::scan(&mut catalog, &root, ctl) {
                    Ok(s) => s,
                    // A refused folder is reported and the others are still scanned; any other error stops the command.
                    Err(pg_scan::ScanError::Refused(reason)) => {
                        println!("{}", text::notice_line(&Notice::RootRefused { root: root.clone(), reason }, rules));
                        continue;
                    }
                    Err(pg_scan::ScanError::Failed(e)) => return Err(e),
                };
                let mut report = scan_report(&root, &s, rules, start.elapsed().as_secs_f64()).into_iter();
                if let Some(summary) = report.next() {
                    println!("{summary}");
                }
                print_failures(Kept::Skipped, &s.failures);
                // What the pairing replaced loses its thumbnails and the temp files of its queued writes (decision 7).
                // The cache is opened only then.
                if !s.pairing.replaced_ids.is_empty() {
                    let (events, printer) = thumb_events();
                    let cache = pg_library::open_thumbs(&layout, &catalog.catalog_id()?, None, events)?;
                    for (_, e) in pg_library::scans::forget_replaced(&cache, &s.pairing.replaced_ids) {
                        say(&Line::ThumbNotForgotten { root: name.clone(), error: format!("{e:#}") });
                    }
                    drop(cache);
                    let _ = printer.join();
                }
                for (temp, e) in pg_writeback::discard_temps(rules, &s.pairing.temps) {
                    say(&Line::TempNotDeleted { root: name.clone(), temp: rules.display(&temp), error: e.to_string() });
                }
                for notice in report {
                    println!("{notice}");
                }
            }
        }
        Command::Remove { root } => {
            // A folder deleted from disk can't be normalized, and it is the one to remove: its path as given is then
            // the catalog's, and the catalog says whether it is a library folder.
            let root = match pg_platform::names::normalize_path(&root) {
                Ok(path) => path,
                Err(_) => pg_platform::names::rule_path_in(rules, &root)
                    .ok_or_else(|| anyhow!(text::render(&Message::NotAPath { path: shown(&root) })))?,
            };
            let name = rules.display(&root);
            let removed = match catalog.root_id(&root)? {
                Some(id) => catalog.remove_root(id)?,
                None => None,
            };
            match removed {
                Some(n) => say(&Line::Removed { root: name, hidden: n.to_string() }),
                None => bail!(line(&Line::NotInLibrary { root: name })),
            }
        }
        Command::Import { pipeline, retry_errors, again } => {
            if !again.is_empty() {
                let marked = catalog.mark_pending(&files_named(&catalog, &again)?)?;
                say(&Line::MarkedAgain { count: marked.to_string() });
            }
            let opts = pipeline.options(Priority::Normal);
            let s = pg_import::import(&mut catalog, &opts, retry_errors, ctl)?;
            say(&Line::ImportPending { pending: s.pending.to_string(), indexed: s.indexed.to_string() });
            say(&Line::ImportDone {
                files: s.files.to_string(),
                failed: s.failed.to_string(),
                changed: changed_while_read(s.changed),
                pairs: s.pairs.to_string(),
                seconds: secs(start.elapsed().as_secs_f64()),
            });
        }
        Command::Thumbs { height, cap, pipeline, foreground } => {
            // Idle: only the cores the user leaves free. I/O priority stays normal: Windows' background mode
            // also throttles reads on an idle disk and made the task 6.5× slower (64 vs 415 files/s on dups2).
            let opts = pipeline.options(if foreground { Priority::Normal } else { Priority::Idle });
            let (events, printer) = thumb_events();
            let cache = pg_library::open_thumbs(&layout, &catalog.catalog_id()?, cap, events)?;
            let s = pg_thumbs::run(&cache, &catalog_path, height, &opts, ctl)?;
            cache.flush();
            say(&Line::ThumbsNeeded {
                needed: s.needed.to_string(),
                height: height.to_string(),
                held: gigabytes(pg_model::num::size(s.cache_bytes)),
            });
            print_failures(Kept::Failed, &s.failures);
            say(&Line::ThumbsDone {
                made: s.made.to_string(),
                height: height.to_string(),
                failed: s.failed.to_string(),
                changed: changed_while_read(s.changed),
                full: if s.cache_full { line(&Line::CacheFull) } else { String::new() },
                used: gigabytes(pg_model::num::signed(cache.totals()?.0)),
                cap: gigabytes(pg_model::num::signed(cache.cap()?)),
                seconds: secs(start.elapsed().as_secs_f64()),
            });
            report_pass(FilePass::Thumbs, s.unreadable, s.ledger_error);
            drop(cache);
            let _ = printer.join();
        }
        Command::Borders { import_review, pipeline, .. } => {
            if let Some(review) = import_review {
                let (marked, unknown) = borders::import_review(&catalog, &review)?;
                say(&Line::ReviewImported { marked: marked.to_string(), unknown: unknown.to_string() });
            } else {
                let s = pg_borders::find(&mut catalog, &pipeline.options(Priority::Normal), ctl)?;
                say(&Line::BordersDone {
                    files: s.files.to_string(),
                    failed: s.failed.to_string(),
                    stored: s.stored.to_string(),
                    seconds: secs(start.elapsed().as_secs_f64()),
                });
                report_pass(FilePass::Borders, s.unreadable, s.ledger_error);
            }
        }
        Command::Status => {
            say(&Line::StatusHome { folder: shown(layout.home()) });
            for root in catalog.roots()? {
                say(&Line::StatusLibrary { folder: rules.display(&root.path) });
            }
            let s = catalog.status()?;
            say(&Line::StatusFiles {
                files: s.files.to_string(),
                missing: s.missing.to_string(),
                analyzed: s.analyzed.to_string(),
                failed: s.failed.to_string(),
                pending: s.pending.to_string(),
                pairs: s.pdq_pairs.to_string(),
            });
            say(&Line::StatusWrites { queued: s.queued_writes.to_string(), failing: s.failed_writes.to_string() });
            match ThumbCache::read_usage(&layout.thumbs_path())? {
                Some(u) => say(&Line::StatusCache {
                    count: u.count.to_string(),
                    used: gigabytes(pg_model::num::signed(u.bytes)),
                    cap: gigabytes(pg_model::num::signed(u.cap)),
                }),
                None => say(&Line::StatusNoCache),
            }
            // The write errors are the writer's own text, listed under the counts.
            for e in catalog.recent_write_errors(5)? {
                println!("  {e}");
            }
        }
        Command::Write => {
            // Only retagged here (never filled): the cap stays as it is.
            // The cache is leaked, so its events print until the process ends.
            let (events, _printer) = thumb_events();
            let cache: &'static ThumbCache =
                Box::leak(Box::new(pg_library::open_thumbs(&layout, &catalog.catalog_id()?, None, events)?));
            let writer = Writer::start(catalog_path.clone(), cache, Arc::new(Bus::new(16)))?;
            writer.wake();
            wait_for_writes(&writer, &catalog)?;
            if let Some(e) = writer.status().thumbnail_error {
                eprintln!("{}", line(&Line::ThumbnailsError { file: e.file.get().to_string(), error: e.error }));
            }
            drop(writer);
            cache.close();
            let s = catalog.status()?;
            say(&Line::WriteDone {
                left: s.queued_writes.to_string(),
                failing: s.failed_writes.to_string(),
                pending: s.pending.to_string(),
                seconds: secs(start.elapsed().as_secs_f64()),
            });
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
        let left = catalog.left_to_write()?;
        if left == 0 {
            return Ok(());
        }
        if !writer.is_running() {
            let left = left.to_string();
            bail!(line(&match writer.status().error {
                Some(error) => Line::WriterStoppedFailing { left, error: error.to_string() },
                None => Line::WriterStopped { left },
            }));
        }
    }
}

/// The present files of the catalog at these paths, each a file or a folder (its files and its subfolders'), each
/// once. Fails, before anything is done, on a path that holds none.
fn files_named(catalog: &Catalog, paths: &[PathBuf]) -> Result<Vec<FileId>> {
    let mut ids = Vec::new();
    for path in paths {
        let rule = normalized(path)?;
        let before = ids.len();
        if path.is_dir() {
            ids.extend(catalog.files_under(&rule)?);
        } else {
            ids.extend(catalog.file_by_path(&rule)?);
        }
        if ids.len() == before {
            bail!(line(&Line::NoFileThere { path: shown(path) }));
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
        eprintln!("{}", line(&Line::BordersServed { count: n.to_string() }));
        return Ok(());
    }
    let csv = csv.with_context(|| line(&Line::CsvMissing))?;
    let s = with_progress(|ctl| borders::report(paths, csv, &pipeline.options(Priority::Normal), ctl))?;
    say(&Line::BordersReport {
        files: s.files.to_string(),
        borders: s.borders.to_string(),
        rejected: s.rejected.to_string(),
        skipped: s.skipped.to_string(),
        errors: s.errors.to_string(),
        seconds: secs(s.seconds),
        rate: format!("{:.0}", pg_model::num::count(s.files) / s.seconds.max(1e-3)),
    });
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

    /// `gallery scan` says what a scan's result carries, as the window's inbox does: a mount the scan refused is a
    /// line of its own, after the counts.
    #[test]
    fn gallery_scan_prints_a_refused_mount() {
        use pg_model::refusal::RefusedReason;

        let rules = PathRules::Posix;
        let (root, mount) = (rules.parse("/photos").unwrap(), rules.parse("/photos/usb").unwrap());
        let reason = RefusedReason::NotNtfs { file_system: "vfat".into() };
        let stats = ScanStats {
            files: 3,
            refused_mount_list: vec![pg_scan::RefusedMount { mount: mount.clone(), reason: reason.clone() }],
            ..ScanStats::default()
        };
        let lines = scan_report(&root, &stats, rules, 1.25);
        let refused = text::notice_line(&Notice::MountRefused { root: root.clone(), mount, reason }, rules);
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert!(lines[0].starts_with("/photos: 3 files in 0 folders"), "{lines:?}");
        assert_eq!(lines[1], refused);
        assert!(refused.contains("/photos/usb"), "{refused}");
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
        let root = pg_platform::names::normalize_path(&dir.join("images")).unwrap();
        pg_scan::scan(&mut catalog, &root, &Control::new()).unwrap();
        state::queue_write(&catalog, state::file_id(&catalog, "a.png").unwrap(), "rating", 3).unwrap();
        let (events, quiet) = pg_jobs_testkit::Quiet::bus();
        let thumbs: &'static ThumbCache = Box::leak(Box::new(
            ThumbCache::open(&dir.join("thumbs.sqlite"), &catalog.catalog_id().unwrap(), Some(1 << 30), 1, events).unwrap(),
        ));
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
