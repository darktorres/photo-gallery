//! What the command line prints besides its reports: the progress lines, the thumbnail cache's events, a
//! rebuilt catalog, and a pass's failures (its kept list, its unreadable files, its ledger's errors).

use std::io;
use std::sync::Arc;
use std::thread::JoinHandle;

use crate::out::stderr_line;
use pg_catalog::Catalog;
use pg_jobs::NoticeSink;
use pg_jobs::control::{Control, Event};
use pg_jobs::events::Bus;
use pg_model::notice::{KeyBuf, Notice, Trigger};
use pg_model::progress::Progress;
use pg_thumbcache::Effect;
use pg_wording::cli::{Line, line};

/// A whole pass's seconds, one decimal: the one rule for seconds on the command line (`lib.rs` uses it too).
pub(crate) fn secs(seconds: f64) -> String {
    format!("{seconds:.1}")
}

/// The thumbnail cache's events, on stderr with the progress as they come. A thread prints them, and ends once the
/// cache that holds the bus is dropped: joining it then means the last one is out. A cache that is leaked (`gallery
/// write`, for the writer's `'static` borrow) never drops, so its thread runs until the process ends and is not joined.
/// When the thread cannot start, the command runs without the lines, and says so. A line the thread cannot write is
/// dropped, not reported: the thread has no caller to return the error to, and the cache's events must still be taken.
pub(crate) fn thumb_events() -> (Arc<Bus<pg_thumbcache::Event>>, Option<JoinHandle<()>>) {
    let bus = Arc::new(Bus::new(64));
    let events = bus.subscribe();
    let printer = std::thread::Builder::new()
        .name("thumbnail printer".into())
        .spawn(move || {
            while let Some(event) = events.recv() {
                if let Some(text) = thumb_event_text(&event) {
                    let _ = stderr_line(text);
                }
            }
        })
        .ok();
    if printer.is_none() {
        let _ = stderr_line("the thumbnail cache's events are not printed: a thread could not be started");
    }
    (bus, printer)
}

/// What a thumbnail cache event says, on the command line: the words of the notice it posts, and for a cache started
/// over, the technical account too. An event that only ends a kept notice (the cache's writer recovered) says nothing
/// here: the command line keeps none.
fn thumb_event_text(event: &pg_thumbcache::Event) -> Option<String> {
    let Some(Effect::Post(notice)) = event.effect() else { return None };
    let line = pg_wording::notice_line(&notice, pg_platform::native::PATH_RULES);
    Some(match notice {
        Notice::CacheStartedOver { detail, .. } => pg_wording::cli::line(&Line::WithDetail { line, detail }),
        _ => line,
    })
}

/// Where the command line's workers (the writer of `gallery write`) post: the words of each notice on stderr, as the
/// cache's events are. It keeps no notices, so a replace or a clear says nothing, and there is nothing to read again.
pub(crate) struct PrintingSink;

impl NoticeSink for PrintingSink {
    fn post(&self, notice: Notice) {
        // The sink's post returns nothing to its writer, so a failed print is dropped (as the events' are).
        let _ = stderr_line(pg_wording::notice_line(&notice, pg_platform::native::PATH_RULES));
    }

    fn replace(&self, _scope: KeyBuf, _notices: Vec<Notice>) {}

    fn resolve(&self, _trigger: Trigger) {}

    fn poke(&self) {}
}

/// Says on stderr that the catalog was rebuilt, why, where the old file went, and what was carried over into the new one
/// (or why nothing was, which says to scan the library again).
pub(crate) fn report_rebuilt(catalog: &Catalog) -> io::Result<()> {
    if let Some(r) = catalog.rebuilt() {
        stderr_line(pg_wording::notice_line(&r.notice(), catalog.rules()))?;
    }
    Ok(())
}
/// Files left for the next pass because they changed while they were read, when there are any (else empty).
pub(crate) fn changed_while_read(n: usize) -> String {
    if n == 0 { String::new() } else { line(&Line::ChangedWhileRead { count: n.to_string() }) }
}

/// Runs `f` with a control whose progress is printed on stderr while it runs: a line every few seconds for each
/// phase, and the phase's last line when it ends. The printer is joined before this returns.
/// When the thread cannot start, the command runs without the progress lines, and says so.
pub(crate) fn with_progress<T>(f: impl FnOnce(&Control) -> T) -> T {
    let ctl = Control::new();
    let events = ctl.subscribe();
    let printer = std::thread::Builder::new()
        .name("progress printer".into())
        .spawn(move || {
            while let Some(event) = events.recv() {
                match event {
                    // The thread has no caller to return a failed print to: the line is dropped, and the work goes on.
                    Event::Progress(p) | Event::Finished(p) => {
                        let _ = stderr_line(progress_line(&p));
                    }
                }
            }
        })
        .ok();
    if printer.is_none() {
        let _ = stderr_line("the progress is not printed: a thread could not be started");
    }
    let out = f(&ctl);
    drop(ctl);
    join_printer(printer);
    out
}

/// Joins a printer thread once its events are gone (none, when it could not start). A printer that panicked is a bug,
/// so its panic is raised again here rather than dropped with the join's error.
pub(crate) fn join_printer(printer: Option<JoinHandle<()>>) {
    if let Some(printer) = printer
        && let Err(panic) = printer.join()
    {
        std::panic::resume_unwind(panic);
    }
}

/// The command line's progress line for a phase.
fn progress_line(p: &Progress) -> String {
    let elapsed = p.elapsed.as_secs_f64().max(1e-3);
    // The pass by the client's words, a scanned folder under this machine's rules (the command's catalogs are made
    // under them).
    let label = pg_wording::pass_name(&p.phase, pg_platform::native::PATH_RULES);
    let words = if p.total == 0 {
        line(&Line::ProgressCount {
            pass: label,
            done: p.done.to_string(),
            rate: format!("{:.0}", p.rate()),
            seconds: secs(elapsed),
        })
    } else {
        line(&Line::Progress {
            pass: label,
            done: p.done.to_string(),
            total: p.total.to_string(),
            percent: format!("{:.1}", 100.0 * p.fraction()),
            rate: format!("{:.0}", p.rate()),
            // A rate, so decimal megabytes per second.
            mbps: format!("{:.0}", pg_model::num::size(p.bytes) / elapsed / 1e6),
            failed: p.failed.to_string(),
            seconds: secs(elapsed),
            eta: secs(p.eta()),
        })
    };
    format!("  {words}")
}

/// What a pass's failures leave besides its kept list, on stderr, in the notices' words: the files it couldn't read,
/// and a failure ledger it couldn't write.
pub(crate) fn report_pass(
    pass: pg_model::notice::FilePass,
    unreadable: usize,
    drives: Vec<String>,
    ledger_error: Option<String>,
) -> io::Result<()> {
    let rules = pg_platform::native::PATH_RULES;
    if unreadable > 0 {
        let count = u64::try_from(unreadable).unwrap_or(u64::MAX);
        stderr_line(pg_wording::notice_line(&Notice::UnreadableFiles { pass, count, drives }, rules))?;
    }
    if let Some(error) = ledger_error {
        stderr_line(pg_wording::notice_line(&Notice::LedgerNotWritten { error }, rules))?;
    }
    Ok(())
}

/// What a pass's kept failures were: files a scan skipped, or files a pass failed on.
#[derive(Clone, Copy)]
pub(crate) enum Kept {
    Skipped,
    Failed,
}

/// The first failures a pass kept, one per line on stderr.
pub(crate) fn print_failures(kept: Kept, failures: &[pg_model::notice::FileFailure]) -> io::Result<()> {
    for f in failures {
        let (path, error) = (pg_platform::native::PATH_RULES.display(&f.path), f.error.clone());
        let words = match kept {
            Kept::Skipped => Line::Skipped { path, error },
            Kept::Failed => Line::Failed { path, error },
        };
        stderr_line(line(&words))?;
    }
    Ok(())
}
