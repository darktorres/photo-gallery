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

use pg_client::text::cli::{Line, line};
use pg_client::text::{self, Message};
use pg_library::folders::{AddError, AddFinishError, Finished};
use pg_model::ids::FileId;
use pg_model::notice::{FilePass, Notice};
use pg_model::rules::{Path as RulePath, PathRules};
use pg_platform::names::{NormalizeError, shown};
use pg_scan::ScanStats;
use serde_json::json;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result, anyhow, bail};
use clap::{Parser, Subcommand};
use pg_catalog::{Catalog, VolumeSet};
use pg_library::lanes::{self, Group, run_each};
use pg_platform::priority::Priority;
use pg_thumbcache::ThumbCache;

use pg_jobs::control::Control;
use pg_jobs::events::Bus;
use pg_writeback::Writer;

use args::{Cli, Command, PipelineArgs};
use report::{
    Kept, PrintingSink, changed_while_read, join_printer, print_failures, report_pass, report_rebuilt, secs,
    thumb_events, with_progress,
};

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

/// The line a build without the app window prints when it is started with no command (the words are `cli.ftl`'s).
pub fn no_window_line() -> String {
    line(&Line::NoWindow)
}

/// Run a command. `args` includes the program name.
pub fn main(args: impl IntoIterator<Item = OsString>) -> Result<()> {
    let cli = Cli::parse_from(args);
    // The table of a perf log: no catalog, and no log of its own.
    if let Command::Perf { file } = &cli.command {
        print!("{}", pg_perf::summarize(file)?);
        return Ok(());
    }
    // `PG_PERF_LOG`, read before anything else runs, so the whole command is in the log. The command is the root span.
    let perf_file = open_perf_log(&cli);
    let name = command_name(&cli.command);
    let outcome = {
        let _root = pg_perf::span(name);
        run(cli)
    };
    if let Some(file) = perf_file {
        let dropped = pg_perf::dropped();
        if dropped > 0 {
            eprintln!("perf log: {dropped} record(s) could not be written to {}", file.display());
        }
        match pg_perf::summarize_session(&file, pg_perf::session_id()) {
            Ok(table) => print!("{table}"),
            Err(e) => eprintln!("perf log: can't read {}: {e}", file.display()),
        }
    }
    outcome
}

/// The command as the perf log names its root span: `cmd.` and the subcommand's name.
fn command_name(command: &Command) -> &'static str {
    match command {
        Command::Scan { .. } => "cmd.scan",
        Command::Remove { .. } => "cmd.remove",
        Command::Import { .. } => "cmd.import",
        Command::Hash { .. } => "cmd.hash",
        Command::Pair { .. } => "cmd.pair",
        Command::Thumbs { .. } => "cmd.thumbs",
        Command::Borders { .. } => "cmd.borders",
        Command::Write => "cmd.write",
        Command::Status => "cmd.status",
        Command::Dups { .. } => "cmd.dups",
        Command::Perf { .. } => "cmd.perf",
    }
}

/// Opens the perf log `PG_PERF_LOG` names, for this run. `-` echoes each top-level step to stderr as it ends; any other
/// value is a file to append to, whose table is printed when the run ends. Unset (or a file that can't be opened, which
/// is said), nothing is recorded. Returns the file, when there is one to summarize.
fn open_perf_log(cli: &Cli) -> Option<std::path::PathBuf> {
    let target = pg_platform::env::read(pg_platform::env::Var::PerfLog)?;
    let meta = json!({
        "program": pg_perf::PROGRAM,
        "version": env!("CARGO_PKG_VERSION"),
        "command": command_name(&cli.command),
        "cores": std::thread::available_parallelism().map_or(0, std::num::NonZero::get),
        "catalog_dir": cli.layout().ok().map(|l| l.catalog_dir().display().to_string()),
    });
    if target.to_str() == Some("-") {
        if let Err(e) = pg_perf::open_echo(|line| eprintln!("{line}"), meta) {
            eprintln!("perf log: {e}");
        }
        return None;
    }
    let path = std::path::PathBuf::from(target);
    match pg_perf::open(&path, meta) {
        Ok(()) => Some(path),
        Err(e) => {
            eprintln!("perf log: can't open {}: {e}; not recording", path.display());
            None
        }
    }
}

/// The command, run: the borders of pixels or of files outside the library need no catalog, and the rest run with one.
fn run(cli: Cli) -> Result<()> {
    // Borders of pixels piped in, or of files outside the library: no catalog.
    if let Command::Borders { paths, csv, stdin, pipeline, .. } = &cli.command
        && (*stdin || !paths.is_empty())
    {
        return borders_outside(paths, csv.as_deref(), *stdin, pipeline);
    }
    let layout = cli.layout()?;
    layout.ensure_catalog_dir(cli.create).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => anyhow!("{e} (--create makes the folder)"),
        _ => e.into(),
    })?;
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

/// What finishing a removal over a folder deleted, and what it kept for an unfinished write.
fn say_finished(finished: &Finished) {
    say(&Line::Removed { root: finished.folder.clone(), deleted: finished.driven.files.to_string() });
    if finished.driven.deferred > 0 {
        say(&Line::RemovalKept { root: finished.folder.clone(), deferred: finished.driven.deferred.to_string() });
    }
}

/// Seconds as the command line shows them: one decimal.
/// The Hashes pass, and its report: the files stored without their duplicate hashes, made now (`hashes::hash`), and how
/// many of them stored.
fn hash_and_report(
    catalog: &mut Catalog,
    opts: &pg_jobs::pipeline::Options,
    ctl: &Control,
    start: Instant,
) -> Result<()> {
    let s = pg_import::hashes::hash(catalog, opts, &pg_catalog::VolumeSet::All, &|| true, ctl)?;
    if s.files > 0 {
        say(&Line::HashesDone {
            files: s.files.to_string(),
            stored: s.stored.to_string(),
            failed: s.failed.to_string(),
            unreadable: s.unreadable.to_string(),
            seconds: secs(start.elapsed().as_secs_f64()),
        });
    }
    Ok(())
}

/// The duplicate hashes, then (`pair`) the pairing pass, and its report: the files paired, the pairs stored, and the
/// analyses that have no orientation hashes and so are left out (made by an older build).
fn pair_and_report(
    catalog: &mut Catalog,
    opts: &pg_jobs::pipeline::Options,
    ctl: &Control,
    start: Instant,
    pair: bool,
) -> Result<()> {
    // The duplicate hashes an import without them left out come first: the pairs need them.
    hash_and_report(catalog, opts, ctl, start)?;
    if !pair {
        return Ok(());
    }
    let s = pg_import::pairs::pair(catalog, opts, ctl)?;
    say(&Line::PairDone {
        files: s.files.to_string(),
        pairs: s.pairs.to_string(),
        seconds: secs(start.elapsed().as_secs_f64()),
    });
    // The parts are fractions of a second: whole milliseconds.
    let (t, part) = (s.timings, millis);
    say(&Line::PairTimings {
        load: part(t.load),
        store: part(t.store),
        join: part(t.join),
        sort: part(t.joins.sort),
        scan: part(t.joins.scan),
        idle: part(t.joins.starved),
        paused: part(t.joins.paused),
        write: part(t.write),
        commit: part(t.commit),
    });
    if s.legacy > 0 {
        say(&Line::PairsLegacy { count: s.legacy.to_string() });
    }
    Ok(())
}

/// A duration's part of a second as whole milliseconds.
fn millis(d: std::time::Duration) -> String {
    d.as_millis().to_string()
}

/// Bytes of 2^30 (a GiB): the cache's sizes and caps are binary, so the command line shows them in GiB.
const GIB: f64 = 1_073_741_824.0;

/// Bytes as the command line shows them in GiB, two decimals.
fn gibibytes(bytes: f64) -> String {
    format!("{:.2}", bytes / GIB)
}

/// What `gallery scan` says of a root it scanned in `seconds`: its counts, then the notes its result carries
/// (`pg_library::scans::scan_notices`: the folder's conditions, then what the walk did), as the window's inbox words
/// them. Then each refused mount the walk listed, with why, and one line for the refused mounts it didn't list. The
/// folders it couldn't read are listed apart, as the walk's skipped folders (`print_failures`); they are the catalog's
/// folder ledger, which the inbox shows.
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
    let mut lines: Vec<String> =
        std::iter::once(summary).chain(notices.all().map(|n| text::notice_line(n, rules))).collect();
    for refused in &s.refused_mount_list {
        lines.push(line(&Line::RefusedMount {
            mount: rules.display(&refused.mount),
            reason: text::refused_reason(&refused.reason),
        }));
    }
    let listed = u64::try_from(s.refused_mount_list.len()).unwrap_or(u64::MAX);
    let not_listed = s.refused_mounts.saturating_sub(listed);
    if not_listed > 0 {
        lines.push(line(&Line::RefusedMountsNotListed { count: not_listed.to_string() }));
    }
    lines
}

fn run_command(cli: Cli, ctl: &Control) -> Result<()> {
    // Before the match below moves out of `cli.command`: the data folders are read from the whole `cli`.
    // The catalog folder as SQLite opens it (an alias in the home for a folder it can't name).
    let layout = cli.layout()?.for_sqlite()?;
    let catalog_path = layout.catalog_path();
    // The catalog as it opens (its file made or migrated, its checks): timed apart, or it is the command's own time.
    let open = pg_perf::span("cmd.open");
    let mut catalog = pg_library::open_catalog(&layout)?;
    drop(open);
    report_rebuilt(&catalog);
    let rules = catalog.rules();
    let start = Instant::now();

    match cli.command {
        Command::Scan { roots, wait, serial } => {
            let roots: Vec<RulePath> = if roots.is_empty() {
                catalog.roots()?.into_iter().map(|r| r.path).collect()
            } else {
                let mut added = Vec::new();
                for root in roots {
                    // A removal over this folder (begun, or kept for an unfinished write) is finished first: an add over it
                    // is refused until then. Its thumbnails are opened only for that finish.
                    let mut finish = || -> Result<pg_catalog::Driven> {
                        let (events, printer) = thumb_events();
                        let cache = pg_library::open_thumbs(&layout, &catalog.catalog_id()?, None, events)?;
                        let driven = pg_library::removals::finish(&catalog, &cache, &catalog_path, &mut |_| true);
                        drop(cache);
                        join_printer(printer);
                        driven
                    };
                    match pg_library::folders::add_folder_finishing(&catalog, &root, &mut finish) {
                        Ok(added_folder) => {
                            if let Some(finished) = &added_folder.finished {
                                say_finished(finished);
                            }
                            if !added_folder.new {
                                let notice = Notice::FolderAlreadyAdded { folder: added_folder.root.clone() };
                                println!("{}", text::render(&Message::NoticeTitle { notice, rules }));
                            }
                            added.push(added_folder.root);
                        }
                        Err(AddFinishError::Add { error, finished }) => {
                            if let Some(finished) = &finished {
                                say_finished(finished);
                            }
                            match error {
                                // A refused folder is not added; the other folders still are.
                                AddError::Refused { folder, reason } => {
                                    println!("{}", text::notice_line(&Notice::FolderRefused { folder, reason }, rules));
                                }
                                AddError::Path(e) => bail!(path_words(&e)),
                                AddError::Catalog(e) => return Err(e.into()),
                            }
                        }
                        Err(AddFinishError::StillPending(finished)) => {
                            say_finished(&finished);
                            return Err(pg_catalog::Error::RemovalPending { path: finished.folder }.into());
                        }
                        Err(AddFinishError::Finish(e)) => return Err(e),
                    }
                }
                added
            };
            // One lane per drive: each walks its roots, on its own connection; the results are reported in order.
            let mut groups = lanes::groups(rules, &roots, serial);
            // A folder the drive probe can't place (gone, or on an unplugged drive) is still walked: the walk is what says why
            // it can't be read, and where the refusal goes. It is one group of its own, which walks no volume's files.
            let unplaced: Vec<RulePath> =
                roots.iter().filter(|root| !groups.iter().any(|group| group.roots.contains(root))).cloned().collect();
            if !unplaced.is_empty() {
                groups.push(Group { roots: unplaced, volumes: VolumeSet::All, label: String::new() });
            }
            let walked = run_each(&catalog_path, &groups, ctl, |catalog, group, lane| {
                Ok(group
                    .roots
                    .iter()
                    .map(|root| (root.clone(), scan_root(catalog, root, lane, wait)))
                    .collect::<Vec<_>>())
            })?;
            for (root, result) in walked.into_iter().flatten() {
                let name = rules.display(&root);
                let s = match result {
                    Ok(s) => s,
                    // A refused folder is reported and the others are still scanned (its refusal is in the catalog's
                    // folder ledger, until a walk of it can run); any other error stops the command.
                    Err(pg_scan::ScanError::Refused(reason)) => {
                        println!(
                            "{}",
                            text::notice_line(&Notice::FolderRefused { folder: root.clone(), reason }, rules)
                        );
                        continue;
                    }
                    // Another process walks a folder that overlaps this one: said, and the command fails (`--wait`
                    // waits for it instead).
                    Err(pg_scan::ScanError::Busy { root, holder }) => {
                        let root = pg_platform::native::PATH_RULES.display(&root);
                        bail!(line(&Line::ScanBusy { root, holder }))
                    }
                    // Another process removed the folder from the library meanwhile (`gallery remove`, or the app):
                    // said, and the command fails, as the scan asked for didn't happen.
                    Err(pg_scan::ScanError::Removed { root }) => bail!(line(&Line::ScanRemoved { root })),
                    Err(pg_scan::ScanError::Failed(e)) => return Err(e),
                };
                let mut report = scan_report(&root, &s, rules, start.elapsed().as_secs_f64()).into_iter();
                if let Some(summary) = report.next() {
                    println!("{summary}");
                }
                print_failures(Kept::Skipped, &s.failures);
                // What the pairing replaced loses its thumbnails and the temp files of its queued writes.
                // The cache is opened only then.
                if !s.pairing.replaced_ids.is_empty() {
                    let (events, printer) = thumb_events();
                    let cache = pg_library::open_thumbs(&layout, &catalog.catalog_id()?, None, events)?;
                    for (_, e) in pg_library::scans::forget_replaced(&cache, &s.pairing.replaced_ids) {
                        say(&Line::ThumbNotForgotten { root: name.clone(), error: format!("{e:#}") });
                    }
                    drop(cache);
                    join_printer(printer);
                }
                for (temp, e) in pg_writeback::discard_temps(rules, &s.pairing.temps) {
                    say(&Line::TempNotDeleted { root: name.clone(), temp: rules.display(&temp), error: e.to_string() });
                }
                for notice in report {
                    println!("{notice}");
                }
            }
        }
        Command::Remove { root, discard_pending: true } => {
            let root = match pg_platform::names::normalize_path(&root) {
                Ok(path) => path,
                Err(_) => pg_platform::names::real_path_lenient(&root)
                    .map_err(|_| anyhow!(text::render(&Message::NotAPath { path: shown(&root) })))?,
            };
            let name = rules.display(&root);
            let Some(discarded) = catalog.discard_removal(&root)? else {
                bail!(line(&Line::NotInLibrary { root: name }))
            };
            // The temps as the catalog stored them (`PathRules::join`): a stored path that no longer parses is none of ours.
            let temps: Vec<_> = discarded.temps.iter().filter_map(|bytes| rules.parse_bytes(bytes)).collect();
            for (temp, e) in pg_writeback::discard_temps(rules, &temps) {
                say(&Line::TempNotDeleted { root: name.clone(), temp: rules.display(&temp), error: e.to_string() });
            }
            // The discarded files' thumbnails: the sweep drops every entry whose file the catalog no longer has.
            let (events, printer) = thumb_events();
            let cache = pg_library::open_thumbs(&layout, &catalog.catalog_id()?, None, events)?;
            let swept = cache.sweep_orphans(&catalog_path);
            drop(cache);
            join_printer(printer);
            swept?;
            say(&Line::Removed { root: name, deleted: discarded.rows.to_string() });
        }
        Command::Remove { root, discard_pending: false } => {
            // A folder deleted from disk can't be normalized, and it is the one to remove: its real part is resolved and the
            // rest taken as typed, which is the catalog's spelling, and the catalog says whether it is a library folder.
            let root = match pg_platform::names::normalize_path(&root) {
                Ok(path) => path,
                Err(_) => pg_platform::names::real_path_lenient(&root)
                    .map_err(|_| anyhow!(text::render(&Message::NotAPath { path: shown(&root) })))?,
            };
            let name = rules.display(&root);
            // A removal begun by an earlier run (its marker left, the folder already out of the list) is finished here.
            let begun = match catalog.root_id(&root)? {
                Some(id) => catalog.begin_removal(id, &root)?,
                None if catalog.removal_markers()?.iter().any(|m| rules.same(&m.path, &root)) => {
                    pg_catalog::Begun::Pending(root.clone())
                }
                None => bail!(line(&Line::NotInLibrary { root: name })),
            };
            match begun {
                pg_catalog::Begun::Started(_) | pg_catalog::Begun::Pending(_) => {
                    let (events, printer) = thumb_events();
                    let cache = pg_library::open_thumbs(&layout, &catalog.catalog_id()?, None, events)?;
                    let driven = pg_library::removals::finish(&catalog, &cache, &catalog_path, &mut |_| true);
                    drop(cache);
                    join_printer(printer);
                    let driven = driven?;
                    say(&Line::Removed { root: name.clone(), deleted: driven.files.to_string() });
                    if driven.deferred > 0 {
                        say(&Line::RemovalKept { root: name, deferred: driven.deferred.to_string() });
                    }
                }
                pg_catalog::Begun::NotFound | pg_catalog::Begun::Changed => {
                    bail!(line(&Line::NotInLibrary { root: name }))
                }
            }
        }
        Command::Import { pipeline, retry_errors, again, no_pair, no_dups, serial } => {
            if !again.is_empty() {
                let marked = catalog.mark_pending(&files_named(&catalog, &again)?)?;
                say(&Line::MarkedAgain { count: marked.to_string() });
            }
            let opts = pipeline.options(Priority::Normal);
            // One lane per drive: each imports its volumes at once. The duplicates below are the whole library's.
            let groups = lanes::command_groups(&catalog, serial)?;
            let imports = run_each(&catalog_path, &groups, ctl, |catalog, group, lane| {
                pg_import::import_while(catalog, &opts, retry_errors, &group.volumes, &|| true, lane)
            })?;
            for (group, s) in groups.iter().zip(&imports) {
                lane_heading(group, groups.len());
                say(&Line::ImportPending { pending: s.pending.to_string() });
                say(&Line::ImportDone {
                    files: s.files.to_string(),
                    failed: s.failed.to_string(),
                    changed: changed_while_read(s.changed),
                    seconds: secs(start.elapsed().as_secs_f64()),
                });
            }
            // The duplicates are the hashes and the pairs: none with `--no-dups`, no pairs with `--no-pair`.
            if !no_dups {
                pair_and_report(&mut catalog, &opts, ctl, start, !no_pair)?;
            }
        }
        Command::Hash { pipeline } => {
            hash_and_report(&mut catalog, &pipeline.options(Priority::Normal), ctl, start)?;
        }
        Command::Pair { pipeline, again } => {
            if again {
                let count = catalog.pairs_again()?;
                say(&Line::PairsAgain { count: count.to_string() });
            }
            pair_and_report(&mut catalog, &pipeline.options(Priority::Normal), ctl, start, true)?;
        }
        Command::Thumbs { height, cap, pipeline, foreground, serial } => {
            // Idle: only the cores the user leaves free. I/O priority stays normal: Windows' background mode
            // also throttles reads on an idle disk and made the task 6.5× slower (64 vs 415 files/s on dups2).
            let opts = pipeline.options(if foreground { Priority::Normal } else { Priority::Idle });
            let (events, printer) = thumb_events();
            let cache = pg_library::open_thumbs(&layout, &catalog.catalog_id()?, cap, events)?;
            // One lane per drive: each makes its volumes' thumbnails and placeholders, all into the one cache.
            let groups = lanes::command_groups(&catalog, serial)?;
            let runs = run_each(&catalog_path, &groups, ctl, |catalog, group, lane| {
                let s = pg_thumbs::run_while(&cache, &catalog_path, height, &opts, &group.volumes, &|| true, lane)?;
                // The placeholders the thumbnails left owing: after them, as the app's rounds run them.
                let placed =
                    pg_thumbs::placeholders::run_while(catalog, &cache, &opts, &group.volumes, &|| true, lane)?;
                Ok((s, placed))
            })?;
            cache.flush();
            for (group, (s, placed)) in groups.iter().zip(&runs) {
                lane_heading(group, groups.len());
                say(&Line::ThumbsNeeded {
                    needed: s.needed.to_string(),
                    height: height.to_string(),
                    held: gibibytes(pg_model::num::size(s.cache_bytes)),
                });
                print_failures(Kept::Failed, &s.failures);
                say(&Line::ThumbsDone {
                    made: s.made.to_string(),
                    height: height.to_string(),
                    failed: s.failed.to_string(),
                    changed: changed_while_read(s.changed),
                    full: if s.cache_full { line(&Line::CacheFull) } else { String::new() },
                    used: gibibytes(pg_model::num::signed(cache.totals()?.0)),
                    cap: gibibytes(pg_model::num::signed(cache.cap()?)),
                    seconds: secs(start.elapsed().as_secs_f64()),
                });
                let ledger_error = match &s.ledger {
                    pg_thumbs::LedgerOutcome::Failed(error) => Some(error.clone()),
                    pg_thumbs::LedgerOutcome::Untouched | pg_thumbs::LedgerOutcome::Written => None,
                };
                report_pass(FilePass::Thumbs, s.unreadable, lane_drives(group), ledger_error);
                say(&Line::PlaceholdersDone {
                    made: placed.made.to_string(),
                    failed: placed.failed.to_string(),
                    seconds: secs(start.elapsed().as_secs_f64()),
                });
            }
            drop(cache);
            join_printer(printer);
        }
        Command::Borders { import_review, pipeline, .. } => {
            if let Some(review) = import_review {
                let (marked, unknown) = borders::import_review(&catalog, &review)?;
                say(&Line::ReviewImported { marked: marked.to_string(), unknown: unknown.to_string() });
            } else {
                let s = pg_borders::find(
                    &mut catalog,
                    &pipeline.options(Priority::Normal),
                    &pg_catalog::VolumeSet::All,
                    ctl,
                )?;
                say(&Line::BordersDone {
                    files: s.files.to_string(),
                    failed: s.failed.to_string(),
                    stored: s.stored.to_string(),
                    seconds: secs(start.elapsed().as_secs_f64()),
                });
                // The borders' ledger is written with the borders, in their batch: a failure to write it failed the pass.
                report_pass(FilePass::Borders, s.unreadable, Vec::new(), None);
            }
        }
        Command::Status => {
            say(&Line::StatusHome { folder: shown(layout.home()) });
            for root in catalog.roots()? {
                say(&Line::StatusLibrary { folder: rules.display(&root.path) });
            }
            // A removal not finished is only reported here: a read command changes nothing. Running `remove` on the
            // folder, or the app (its removal job, after its first start), finishes it.
            for marker in catalog.removal_markers()? {
                let folder = rules.display(&marker.path);
                say(&Line::StatusRemoving { folder, deferred: marker.deferred.to_string() });
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
            let unhashed = catalog.owed_hashes()?;
            if unhashed > 0 {
                say(&Line::HashesOwed { count: unhashed.to_string() });
            }
            let owed = catalog.owed_pairs()?;
            if owed > 0 {
                say(&Line::PairsOwed { count: owed.to_string() });
            }
            let placeholders = catalog.owed_placeholders()?;
            if placeholders > 0 {
                say(&Line::PlaceholdersOwed { count: placeholders.to_string() });
            }
            say(&Line::StatusWrites { queued: s.queued_writes.to_string(), failing: s.failed_writes.to_string() });
            match ThumbCache::read_usage(&layout.thumbs_path())? {
                Some(u) => say(&Line::StatusCache {
                    count: u.count.to_string(),
                    used: gibibytes(pg_model::num::signed(u.bytes)),
                    cap: gibibytes(pg_model::num::signed(u.cap)),
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
            let writer = Writer::start(catalog_path.clone(), cache, Arc::new(Bus::new(16)), Arc::new(PrintingSink))?;
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
        // `main` reads the perf log itself, before any command runs.
        Command::Perf { .. } => {}
    }
    Ok(())
}

/// Scan `root`; with `wait`, a walk refused because another process walks an overlapping folder is tried again, once a
/// second, until that walk ends (or its lease expires), and the wait is said once on stderr.
/// The heading of a lane's lines when the command runs several: its drive's label. Nothing for one lane.
/// The drive a lane's notices name: its drive's label, when the drive could be told.
fn lane_drives(group: &Group) -> Vec<String> {
    if group.label.is_empty() { Vec::new() } else { vec![group.label.clone()] }
}

fn lane_heading(group: &Group, lanes: usize) {
    if lanes > 1 {
        println!("{}:", group.label);
    }
}

fn scan_root(
    catalog: &mut Catalog,
    root: &RulePath,
    ctl: &Control,
    wait: bool,
) -> std::result::Result<ScanStats, pg_scan::ScanError> {
    let mut said = false;
    loop {
        match pg_scan::scan(catalog, root, ctl) {
            Err(busy @ pg_scan::ScanError::Busy { .. }) if wait && !ctl.stop_requested() => {
                if !said {
                    eprintln!("{busy}: waiting for it to end");
                    said = true;
                }
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
            other => return other,
        }
    }
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

    /// `gallery scan` says what a scan's result carries, as the window's inbox does: each note a line of its own, after
    /// the counts, the folder's conditions before what the walk did. Then each refused mount the walk listed, and one
    /// line for the refused mounts past the list.
    #[test]
    fn gallery_scan_prints_its_notes() {
        use pg_model::notice::ScanNote;
        use pg_model::refusal::RefusedReason;

        let rules = PathRules::Posix;
        let (root, mount) = (rules.parse("/photos").unwrap(), rules.parse("/photos/usb").unwrap());
        let reason = RefusedReason::NotNtfs { file_system: "vfat".into() };
        let mut stats = ScanStats {
            files: 3,
            duplicates: 2,
            refused_mounts: 1,
            refused_mount_list: vec![pg_scan::RefusedMount { mount, reason }],
            ..ScanStats::default()
        };
        stats.pairing.rebound = 1;
        let lines = scan_report(&root, &stats, rules, 1.25);
        let note = |note, count| text::notice_line(&Notice::ScanNote { root: root.clone(), note, count }, rules);
        assert_eq!(lines.len(), 4, "{lines:?}");
        assert!(lines[0].starts_with("/photos: 3 files in 0 folders"), "{lines:?}");
        assert_eq!(lines[1..3], [note(ScanNote::Duplicates, 2), note(ScanNote::Rebound, 1)]);
        assert!(lines[3].starts_with("/photos/usb isn't scanned: "), "{lines:?}");

        // Two refused mounts the walk didn't list: the count says so, after the listed one.
        stats.refused_mounts = 3;
        let more = scan_report(&root, &stats, rules, 1.25);
        assert_eq!(more.len(), 5, "{more:?}");
        assert!(more[4].starts_with("2 more mounts under the scanned folders aren't scanned"), "{more:?}");
    }

    /// `gallery scan` over three library folders, the middle one refused (here, gone since it was added): it is said
    /// and recorded in the folder ledger, and the folders after it are still scanned.
    #[test]
    fn a_refused_folder_between_two_others_does_not_stop_the_scan() {
        let dir = pg_testkit::run_folder_with("scratch_cli", "cli-refused", &["a", "b", "c", "catalog"]).unwrap();
        let png = |folder: &str, name: &str| {
            std::fs::write(dir.join(folder).join(name), pg_testkit::png_rgb(4, 4, &[0; 48]).unwrap()).unwrap();
        };
        for folder in ["a", "b", "c"] {
            png(folder, "1.png");
        }
        let catalog_dir = dir.join("catalog").to_string_lossy().into_owned();
        let gallery = |more: &[String]| {
            let mut args = vec!["gallery".to_string(), "--catalog-dir".to_string(), catalog_dir.clone(), "scan".into()];
            args.extend_from_slice(more);
            Cli::try_parse_from(args).unwrap()
        };
        let folders: Vec<String> = ["a", "b", "c"].iter().map(|f| dir.join(f).to_string_lossy().into_owned()).collect();
        run_command(gallery(&folders), &Control::new()).unwrap();

        let b = pg_platform::names::normalize_path(&dir.join("b")).unwrap();
        // The new files are made before b is removed: on Linux a file made after a removal can take the removed file's
        // inode, and then it is taken for b's row (the folder is refused, not walked, so that row stays and is present).
        png("a", "2.png");
        png("c", "2.png");
        std::fs::remove_dir_all(dir.join("b")).unwrap();
        run_command(gallery(&[]), &Control::new()).unwrap();

        let layout = gallery(&[]).layout().unwrap();
        let catalog = Catalog::open(&layout.catalog_path(), pg_platform::native::PATH_RULES).unwrap();
        // a and c with their new files, and b's file kept as it was (a refused folder is not walked).
        assert_eq!(catalog.status().unwrap().files, 5);
        let refused = catalog.failed_folders(10).unwrap();
        assert_eq!(
            refused.iter().map(|f| (&f.path, f.failure)).collect::<Vec<_>>(),
            [(&b, pg_catalog::FolderFailure::Refused)]
        );
    }

    /// C1: `scan`, `import` and `thumbs` over two roots give the same catalog with `--serial` and through the lanes. The
    /// scratch folders share one volume, so both runs are one lane; what the test holds is that the lane path does what
    /// the serial path does, file for file, in both catalogs.
    #[test]
    fn two_roots_scan_import_and_thumbs_agree_with_and_without_serial() {
        let dir = pg_testkit::run_folder_with("scratch_cli", "cli-serial", &["a", "b", "serial", "lanes"]).unwrap();
        for folder in ["a", "b"] {
            for name in ["1.png", "2.png"] {
                std::fs::write(dir.join(folder).join(name), pg_testkit::png_rgb(4, 4, &[0; 48]).unwrap()).unwrap();
            }
        }
        let folders: Vec<String> = ["a", "b"].iter().map(|f| dir.join(f).to_string_lossy().into_owned()).collect();
        let run = |catalog: &str, serial: bool| {
            let catalog_dir = dir.join(catalog).to_string_lossy().into_owned();
            let command = |verb: Vec<String>| {
                let mut all = vec!["gallery".to_string(), "--catalog-dir".to_string(), catalog_dir.clone()];
                all.extend(verb);
                if serial {
                    all.push("--serial".to_string());
                }
                Cli::try_parse_from(all).unwrap()
            };
            let mut scan = vec!["scan".to_string()];
            scan.extend(folders.iter().cloned());
            run_command(command(scan), &Control::new()).unwrap();
            run_command(command(vec!["import".to_string()]), &Control::new()).unwrap();
            run_command(command(vec!["thumbs".into(), "--height".into(), "64".into()]), &Control::new()).unwrap();

            let layout = command(vec!["scan".to_string()]).layout().unwrap();
            let catalog = Catalog::open(&layout.catalog_path(), pg_platform::native::PATH_RULES).unwrap();
            let s = catalog.status().unwrap();
            (s.files, s.missing, s.analyzed, s.failed, s.pending)
        };
        let serial = run("serial", true);
        let lanes = run("lanes", false);
        assert_eq!(serial, lanes, "the lane path and --serial give the same catalog");
        assert_eq!(serial.0, 4, "the four files of the two roots are in the catalog");
        assert_eq!(serial.2, 4, "and each was imported");
    }

    /// `gallery remove` takes the folder's kept notices with it (the notices' folder is a foreign key that cascades),
    /// and the other folder's stay.
    #[test]
    fn gallery_remove_takes_the_folders_kept_notices() {
        use pg_model::notice::ScanNote;

        let dir = pg_testkit::run_folder_with("scratch_cli", "cli-remove", &["a", "b", "catalog"]).unwrap();
        for folder in ["a", "b"] {
            std::fs::write(dir.join(folder).join("1.png"), pg_testkit::png_rgb(4, 4, &[0; 48]).unwrap()).unwrap();
        }
        let catalog_dir = dir.join("catalog").to_string_lossy().into_owned();
        let gallery = |args: &[&str]| {
            let mut words = vec!["gallery", "--catalog-dir", catalog_dir.as_str()];
            words.extend_from_slice(args);
            Cli::try_parse_from(words).unwrap()
        };
        let (a, b) = (dir.join("a").to_string_lossy().into_owned(), dir.join("b").to_string_lossy().into_owned());
        run_command(gallery(&["scan", &a, &b]), &Control::new()).unwrap();

        let rules = pg_platform::native::PATH_RULES;
        let root = |folder: &str| pg_platform::names::normalize_path(&dir.join(folder)).unwrap();
        let (root_a, root_b) = (root("a"), root("b"));
        let note = |root: &RulePath| Notice::ScanNote { root: root.clone(), note: ScanNote::Duplicates, count: 1 };
        let layout = gallery(&["scan"]).layout().unwrap();
        let catalog = Catalog::open(&layout.catalog_path(), rules).unwrap();
        catalog.post_notice(&note(&root_a), 0).unwrap();
        catalog.post_notice(&note(&root_b), 0).unwrap();
        assert_eq!(catalog.inbox_entries(10, false).unwrap().len(), 2);
        drop(catalog);

        run_command(gallery(&["remove", &a]), &Control::new()).unwrap();

        let catalog = Catalog::open(&layout.catalog_path(), rules).unwrap();
        let kept: Vec<Notice> = catalog.inbox_entries(10, false).unwrap().into_iter().map(|e| e.notice).collect();
        assert_eq!(kept, [note(&root_b)]);
    }

    /// A folder added again over a removal begun and left unfinished (as a stopped app leaves one): the scan finishes the
    /// removal first, then adds the folder and scans it.
    #[test]
    fn gallery_scan_finishes_a_removal_over_the_folder_first() {
        readd_over_a_removal_typed_as(|folder| Some(folder.to_path_buf()));
    }

    /// The same, the folder typed with a `..` that goes out of it and back in: a spelling of the same folder.
    #[test]
    fn gallery_scan_finishes_a_removal_typed_with_dot_dot() {
        readd_over_a_removal_typed_as(|folder| Some(folder.join("..").join(folder.file_name()?)));
    }

    /// The same, the folder typed by its 8.3 short name: the spelling the failing run had, from a temp folder with a long
    /// name on a volume with short names. Skipped where the volume has none.
    #[cfg(windows)]
    #[test]
    fn gallery_scan_finishes_a_removal_typed_by_its_short_name() {
        readd_over_a_removal_typed_as(pg_platform::testing::short_path);
    }

    /// A folder added again over a removal begun and left unfinished (as a stopped app leaves one), typed as `spelling`
    /// gives it: the scan finishes the removal first, then adds the folder and scans it.
    fn readd_over_a_removal_typed_as(spelling: impl Fn(&Path) -> Option<PathBuf>) {
        let dir = pg_testkit::run_folder_with("scratch_cli", "cli-readd", &["a", "catalog"]).unwrap();
        let Some(typed) = spelling(&dir.join("a")) else {
            pg_testkit::skip("this volume gives the folder no other spelling: short names are off");
            return;
        };
        std::fs::write(dir.join("a").join("1.png"), pg_testkit::png_rgb(4, 4, &[0; 48]).unwrap()).unwrap();
        let catalog_dir = dir.join("catalog").to_string_lossy().into_owned();
        let gallery = |args: &[&str]| {
            let mut words = vec!["gallery", "--catalog-dir", catalog_dir.as_str()];
            words.extend_from_slice(args);
            Cli::try_parse_from(words).unwrap()
        };
        let a = dir.join("a").to_string_lossy().into_owned();
        run_command(gallery(&["scan", &a]), &Control::new()).unwrap();

        let rules = pg_platform::native::PATH_RULES;
        let root_a = pg_platform::names::normalize_path(&dir.join("a")).unwrap();
        let layout = gallery(&["scan"]).layout().unwrap();
        let catalog = Catalog::open(&layout.catalog_path(), rules).unwrap();
        let id = catalog.root_id(&root_a).unwrap().unwrap();
        assert!(matches!(catalog.begin_removal(id, &root_a).unwrap(), pg_catalog::Begun::Started(_)));
        assert_eq!(catalog.removal_markers().unwrap().len(), 1);
        drop(catalog);

        run_command(gallery(&["scan", &typed.to_string_lossy()]), &Control::new()).unwrap();

        let catalog = Catalog::open(&layout.catalog_path(), rules).unwrap();
        assert!(catalog.removal_markers().unwrap().is_empty(), "the removal was finished first");
        let roots = catalog.roots().unwrap();
        assert_eq!(roots.len(), 1, "the folder is in the library again");
        assert!(rules.same(&roots[0].path, &root_a));
        assert_eq!(catalog.status().unwrap().files, 1, "the folder is scanned again");
    }

    #[test]
    fn a_missing_catalog_folder_is_refused_with_the_create_hint_unless_asked() {
        let dir = pg_testkit::run_folder("scratch_cli", "missing-catalog-folder").unwrap();
        let archive = dir.join("elsewhere").join("archive");
        let words = |extra: &[&str]| {
            let mut all = vec!["gallery", "--catalog-dir", archive.to_str().unwrap()];
            all.extend_from_slice(extra);
            all.push("status");
            Cli::try_parse_from(all).unwrap()
        };
        let err = run(words(&[])).unwrap_err();
        assert!(format!("{err:#}").contains("--create makes the folder"), "{err:#}");
        assert!(!dir.join("elsewhere").exists(), "nothing is made without --create");
        run(words(&["--create"])).unwrap();
        assert!(archive.is_dir(), "with --create it is made");
        let _ = std::fs::remove_dir_all(&dir);
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

        let dir = pg_testkit::run_folder_with("scratch_cli", "cli-write", &["images"]).unwrap();
        std::fs::write(dir.join("images").join("a.png"), pg_testkit::png_rgb(4, 4, &[0; 48]).unwrap()).unwrap();
        let catalog_path = dir.join("catalog.sqlite");
        let mut catalog =
            Catalog::prepare(&catalog_path, pg_library::versions(), pg_platform::native::PATH_RULES).unwrap();
        let root = pg_platform::names::normalize_path(&dir.join("images")).unwrap();
        catalog.add_root(&root).unwrap();
        pg_scan::scan(&mut catalog, &root, &Control::new()).unwrap();
        state::queue_write(&catalog, state::file_id(&catalog, "a.png").unwrap(), "rating", 3).unwrap();
        let (events, quiet) = pg_jobs_testkit::Quiet::bus();
        let thumbs: &'static ThumbCache = Box::leak(Box::new(
            ThumbCache::open(
                &dir.join("thumbs.sqlite"),
                &catalog.catalog_id().unwrap(),
                pg_thumbcache::Cap::from_bytes(1 << 30),
                pg_library::versions().thumbs,
                events,
            )
            .unwrap(),
        ));
        drop(catalog);
        let sink: Arc<dyn pg_jobs::NoticeSink> = pg_jobs_testkit::RecordingSink::new();
        let writer =
            Writer::start_with_fault(catalog_path.clone(), thumbs, Arc::new(Bus::new(16)), Fault::PanicInDrain, sink)
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
        assert!(error.contains("writing into the files has stopped with 1 changes still to write"), "{error}");
        quiet.check();
    }
}
