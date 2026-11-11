//! What the command line prints besides its reports: the progress lines, the thumbnail cache's events, a
//! rebuilt catalog, and a pass's failures (its kept list, its unreadable files, its ledger's errors).

use std::sync::Arc;
use std::thread::JoinHandle;

use pg_catalog::Catalog;
use pg_client::text;
use pg_client::text::cli::{Line, line};
use pg_jobs::control::{Control, Event};
use pg_jobs::events::Bus;
use pg_model::notice::Notice;
use pg_model::progress::Progress;

/// The thumbnail cache's events, on stderr with the progress as they come. A thread prints them, and ends once the
/// cache that holds the bus is dropped: joining it then means the last one is out. A cache that is leaked (`gallery
/// write`, for the writer's `'static` borrow) never drops, so its thread runs until the process ends and is not joined.
pub(crate) fn thumb_events() -> (Arc<Bus<pg_thumbcache::Event>>, JoinHandle<()>) {
    let bus = Arc::new(Bus::new(64));
    let events = bus.subscribe();
    let printer = std::thread::spawn(move || {
        while let Some(event) = events.recv() {
            eprintln!("{}", thumb_event_text(&event));
        }
    });
    (bus, printer)
}

/// What a thumbnail cache event says, on the command line: its notice's words, and for a cache started over, the
/// technical account too.
fn thumb_event_text(event: &pg_thumbcache::Event) -> String {
    let notice = event.clone().into_notice();
    let line = text::notice_line(&notice, pg_platform::native::PATH_RULES);
    match notice {
        Notice::CacheStartedOver { detail, .. } => text::cli::line(&Line::WithDetail { line, detail }),
        _ => line,
    }
}

/// Says on stderr that the catalog was rebuilt, why, where the old file went, and what was carried over into the new one
/// (or why nothing was, which says to scan the library again).
pub(crate) fn report_rebuilt(catalog: &Catalog) {
    if let Some(r) = catalog.rebuilt() {
        eprintln!("{}", text::notice_line(&r.notice(), catalog.rules()));
    }
}
/// Files left for the next pass because they changed while they were read, when there are any (else empty).
pub(crate) fn changed_while_read(n: usize) -> String {
    if n == 0 { String::new() } else { line(&Line::ChangedWhileRead { count: n.to_string() }) }
}

/// Runs `f` with a control whose progress is printed on stderr while it runs: a line every few seconds for each
/// phase, and the phase's last line when it ends. The printer is joined before this returns.
pub(crate) fn with_progress<T>(f: impl FnOnce(&Control) -> T) -> T {
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
    let secs = p.elapsed.as_secs_f64().max(1e-3);
    // The pass by the client's words, a scanned folder under this machine's rules (the command's catalogs are made
    // under them).
    let label = text::pass_name(&p.phase, pg_platform::native::PATH_RULES);
    let words = if p.total == 0 {
        line(&Line::ProgressCount {
            pass: label,
            done: p.done.to_string(),
            rate: format!("{:.0}", p.rate()),
            seconds: format!("{secs:.0}"),
        })
    } else {
        line(&Line::Progress {
            pass: label,
            done: p.done.to_string(),
            total: p.total.to_string(),
            percent: format!("{:.1}", 100.0 * p.fraction()),
            rate: format!("{:.0}", p.rate()),
            mbps: format!("{:.0}", pg_model::num::size(p.bytes) / secs / 1e6),
            failed: p.failed.to_string(),
            seconds: format!("{secs:.0}"),
            eta: format!("{:.0}", p.eta()),
        })
    };
    format!("  {words}")
}

/// What a pass's failures leave besides its kept list, on stderr, in the notices' words: the files it couldn't read,
/// and a failure ledger it couldn't write.
pub(crate) fn report_pass(pass: pg_model::notice::FilePass, unreadable: usize, ledger_error: Option<String>) {
    let rules = pg_platform::native::PATH_RULES;
    if unreadable > 0 {
        let count = u64::try_from(unreadable).unwrap_or(u64::MAX);
        eprintln!("{}", text::notice_line(&Notice::UnreadableFiles { pass, count }, rules));
    }
    if let Some(error) = ledger_error {
        eprintln!("{}", text::notice_line(&Notice::LedgerNotWritten { pass, error }, rules));
    }
}

/// What a pass's kept failures were: files a scan skipped, or files a pass failed on.
#[derive(Clone, Copy)]
pub(crate) enum Kept {
    Skipped,
    Failed,
}

/// The first failures a pass kept, one per line on stderr.
pub(crate) fn print_failures(kept: Kept, failures: &[pg_model::notice::FileFailure]) {
    for f in failures {
        let (path, error) = (f.path.clone(), f.error.clone());
        let words = match kept {
            Kept::Skipped => Line::Skipped { path, error },
            Kept::Failed => Line::Failed { path, error },
        };
        eprintln!("{}", line(&words));
    }
}
