//! What the command line prints besides its reports: the progress lines, the thumbnail cache's events, a
//! rebuilt catalog, and a pass's failures.

use std::sync::Arc;
use std::thread::JoinHandle;

use pg_catalog::Catalog;
use pg_client::text::{self, CacheStartOver};
use pg_jobs::control::{Control, Event, Phase, Progress};
use pg_jobs::events::Bus;

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

/// What a thumbnail cache event says, on the command line.
fn thumb_event_text(event: &pg_thumbcache::Event) -> String {
    use pg_thumbcache::Event;
    match event {
        Event::TotalsFixed { kept, actual } => {
            format!("thumbnail cache: totals were {kept:?}, the entries hold {actual:?}; fixed")
        }
        Event::TouchesNotWritten { error } => format!("thumbnail cache: touches not written yet: {error}"),
        Event::NotStored { error } => format!("thumbnail cache: a thumbnail not stored: {error}"),
        Event::WalNotMerged { error } => format!("thumbnail cache: the WAL was not merged at close: {error}"),
        Event::StartedOver { reason, path, detail } => format!(
            "{}; detail: {detail}",
            text::thumb_cache_started_over(started_over_reason(*reason), &path.display().to_string())
        ),
    }
}

/// The client's words for why the thumbnail cache was started over.
fn started_over_reason(reason: pg_thumbcache::StartOverReason) -> CacheStartOver {
    use pg_thumbcache::StartOverReason as Reason;
    match reason {
        Reason::Damaged => CacheStartOver::Damaged,
        Reason::NotADatabase => CacheStartOver::NotADatabase,
        Reason::OtherCatalog => CacheStartOver::OtherCatalog,
        Reason::OtherBuild => CacheStartOver::OtherBuild,
    }
}

/// Says on stderr that the catalog was rebuilt, why, where the old file went, and what was carried over into the new one
/// (or why nothing was, which says to scan the library again).
pub(crate) fn report_rebuilt(catalog: &Catalog) {
    if let Some(r) = catalog.rebuilt() {
        eprintln!("catalog rebuilt: {}; the old one is kept as {}{}", r.why, r.kept.display(), r.carried_note());
    }
}

/// Files left for the next pass because they changed while they were read, when there are any.
pub(crate) fn changed_while_read(n: usize) -> String {
    if n == 0 { String::new() } else { format!(", {n} changed while read, left for the next pass") }
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
    let secs = p.elapsed().as_secs_f64().max(1e-3);
    let label = phase_label(&p.phase);
    if p.total == 0 {
        return format!("  {label}: {} files, {:.0} files/s, {secs:.0} s", p.done, p.rate());
    }
    format!(
        "  {label}: {}/{} ({:.1}%), {:.0} files/s, {:.0} MB/s, {} failed, {secs:.0} s elapsed, ETA {:.0} s",
        p.done,
        p.total,
        100.0 * p.fraction(),
        p.rate(),
        pg_model::num::size(p.bytes) / secs / 1e6,
        p.failed,
        p.eta()
    )
}

fn phase_label(phase: &Phase) -> String {
    match phase {
        Phase::Idle => "idle".into(),
        Phase::Crop => "crop".into(),
        Phase::Scan { root } => format!("scan {root}"),
        Phase::Import => "import".into(),
        Phase::Borders => "borders".into(),
        Phase::Thumbs { height } => format!("thumbs {height} px"),
    }
}

/// The first failures a pass kept, one per line on stderr, under `what`.
pub(crate) fn print_failures(what: &str, failures: &[pg_jobs::failure::Failure]) {
    for f in failures {
        eprintln!("{what}: {}: {}", f.path, f.error);
    }
}
