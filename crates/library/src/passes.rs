//! The passes of a lane's round, in order: the folders asked for are walked (`Scans`), the pictures they hold are imported,
//! their thumbnails and placeholders made, their duplicates hashed and paired, and the borders looked for. Each pass works
//! over the lane's volumes (`Inner::volume_set`), so the lanes of two drives run these passes at once, and the lanes of
//! one drive run them in turn.

use std::sync::{Mutex, PoisonError};

use pg_catalog::Catalog;
use pg_jobs::failure::Failed;
use pg_jobs::schedule::{Pass, Step};
use pg_model::ids::FileId;
use pg_model::notice::{ERROR_CAP, FileFailure, FilePass, Key, Notice, Op, Trigger, tidy};
use pg_model::rules::{Path as RulePath, PathRules};
use pg_scan::ScanError;

use crate::Error;
use crate::inner::Inner;
use crate::scans;
use crate::shared::Shared;
use crate::tuning::TunedPass;

/// A lane's pass context: the lane, its round's connection, and whether a pass of the round failed with an error it said
/// with a notice of its own (the thumbnail cache's states, section 10 D7): the round is then a failed one for the backoff,
/// and posts no `RoundFailed` for it.
pub(crate) struct Cx<'a> {
    pub(crate) inner: &'a Inner,
    pub(crate) catalog: &'a mut Catalog,
    pub(crate) said: &'a mut bool,
}

/// A file failure of a crop job, with its error capped as a notice holds it.
pub(crate) fn cap_error(failure: FileFailure) -> FileFailure {
    FileFailure { error: tidy(&failure.error, ERROR_CAP), ..failure }
}

/// An error's text as a notice holds it (`pg_model::notice::error_text`).
pub(crate) use pg_model::notice::error_text;

/// An error's text as a notice holds it, with the causes under it (`pg_model::error::chain`).
pub(crate) fn error_chain(error: &(dyn std::error::Error + 'static)) -> String {
    error_text(&pg_model::error::chain(error))
}

/// Originals were cut in place: their thumbnails are forgotten (they show the uncut pixels), and the import that follows
/// analyzes them again. The app is told the files changed.
pub(crate) fn cut_done(shared: &Shared, cut: &[FileId]) {
    if cut.is_empty() {
        return;
    }
    // The cut itself went through: a thumbnail the cache refused to forget is the cache's failure, counted as its own
    // notice, and not a failed cut of the originals.
    let refused = scans::forget_replaced(&shared.thumbs, cut).len();
    if refused > 0 {
        shared.notices.post(Notice::ThumbnailForgetFailed { count: pg_model::num::count_u64(refused) });
    }
    shared.imported();
}

/// The lost leases of one folder's walks in a row at which the loss is said (section 5): one or two are a walk held up,
/// walked again quietly.
pub(crate) const LOSSES_SAID: u32 = 3;

/// What the pairing of a walk committed, acted on after the commit: the thumbnails of the rows it replaced and of the fresh
/// rows it deleted are forgotten (section 10, D14), and the temp files of the queued writes it dropped are deleted. A
/// failure of either is said as an event (`ScanFailed`, not a walk's).
pub(crate) fn after_pairing(inner: &Inner, rules: PathRules, path: &RulePath, pairing: &pg_catalog::pair::Pairing) {
    let stale: Vec<FileId> = pairing.replaced_ids.iter().chain(&pairing.deleted_ids).copied().collect();
    for (_, e) in scans::forget_replaced(&inner.shared.thumbs, &stale) {
        inner.notify(Notice::ScanFailed { root: path.clone(), error: error_chain(&e), walk: false });
    }
    for (temp, e) in pg_writeback::discard_temps(rules, &pairing.temps) {
        inner.notify(Notice::ScanFailed {
            root: path.clone(),
            error: tidy(&format!("can't delete {}: {e}", rules.display(&temp)), ERROR_CAP),
            walk: false,
        });
    }
}

/// The folder a lane is walking, shown to the coordinator until the guard drops: the mark is cleared however the walk ends,
/// a panic included.
struct Walking<'a>(&'a Mutex<Option<RulePath>>);

impl<'a> Walking<'a> {
    fn begin(slot: &'a Mutex<Option<RulePath>>, path: &RulePath) -> Self {
        *slot.lock().unwrap_or_else(PoisonError::into_inner) = Some(path.clone());
        Self(slot)
    }
}

impl Drop for Walking<'_> {
    fn drop(&mut self) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }
}

/// Walks the folders asked for, skipping those no longer in the library. A scan stopped early goes back to the front.
pub(crate) struct Scans;

impl Pass<Cx<'_>> for Scans {
    fn run(&mut self, cx: &mut Cx<'_>) -> Result<Step, Failed> {
        let shared = &cx.inner.shared;
        let rules = cx.catalog.rules();
        loop {
            let Some(path) = cx.inner.scans().pop_front() else {
                return Ok(Step::Done);
            };
            #[cfg(test)]
            shared.hooks.at_pop(&path);
            // The roots as they are at this pop, not at the pass's start: a folder added while an earlier walk ran is in
            // them (section 2). Roots are few, so this is one indexed read per walk. A read that fails puts the folder back.
            let library: Vec<RulePath> = match cx.catalog.roots() {
                Ok(roots) => roots.into_iter().map(|r| r.path).collect(),
                Err(e) => {
                    cx.inner.push_front_scan(path);
                    return Err(Error::from(e).into());
                }
            };
            // A folder asked for that is no longer in the library is skipped: a removed folder, whose scans the removal
            // stopped.
            if !library.iter().any(|l| rules.is_under(&path, l)) {
                continue;
            }
            let scanned = {
                let _walking = Walking::begin(&cx.inner.scanning, &path);
                pg_scan::scan(cx.catalog, &path, &cx.inner.ctl)
            };
            // What a scan leaves to the runner after the scan's phase has closed: the replaced thumbnails, the temp files, the
            // notices.
            let _after = pg_perf::span("scan.after");
            // What the pairing's committed batches did is acted on however the walk ended (section 5): a walk that failed
            // after them carries them in its error.
            if let Some(pairing) = scanned.as_ref().map_or_else(ScanError::pairing, |s| Some(&s.pairing)) {
                after_pairing(cx.inner, rules, &path, pairing);
            }
            match scanned {
                Ok(s) => {
                    // A scan stopped early has committed what it recorded so far.
                    if s.changed > 0 || s.missing > 0 || s.pairing.rebound > 0 || s.pairing.replaced > 0 {
                        cx.inner.scanned();
                    }
                    let found = scans::scan_notices(&path, &s);
                    // What the walk did is said whatever folder it walked, and stopped or not: the pairing's binds and
                    // replacements are in, and the walk that resumes doesn't do them again.
                    for notice in found.events {
                        cx.inner.notify(notice);
                    }
                    if !s.complete {
                        cx.inner.push_front_scan(path);
                        return Ok(Step::Preempted);
                    }
                    // A complete walk of a whole library folder is what holds for it now: its kept notes (the folder's
                    // conditions) are replaced by the ones it found. A walk of a folder under it looked at part of the
                    // folder only, so it changes none of them. The folders it couldn't read are the folder ledger's,
                    // which the walk itself recorded and swept.
                    // A failed walk of it is a condition of the folder too (section 9): the complete walk clears it.
                    if library.contains(&path) {
                        shared.notices.replace(Key::all("scan-note", Some(&path)).to_buf(), found.conditions);
                        shared.notices.replace(Key::all("scan-failed", Some(&path)).to_buf(), Vec::new());
                        // The request that asked for this walk is done (`Catalog::request_scan`). A request that is not
                        // cleared is walked again at the next start, which is harmless, so the failure is said as an event.
                        if let Err(e) = cx.catalog.finish_scan_request(&path) {
                            cx.inner.notify(Notice::ScanFailed {
                                root: path.clone(),
                                error: error_chain(&e),
                                walk: false,
                            });
                        }
                    }
                    // A complete walk ends the run of lost leases.
                    cx.inner.lease_kept(&path);
                }
                // Refused before any walk: the refusal is in the folder ledger (`pg_scan::scan` recorded it, and poked
                // the inbox), until a walk of the folder can run.
                Err(ScanError::Refused(_)) => {}
                // Another walk of an overlapping folder holds its lease (in this process or another): nothing was
                // written, and the scan is tried again once that walk can have ended.
                Err(ScanError::Busy { .. }) => cx.inner.walk_busy(path),
                // The folder was removed from the library while it was walked (here, or by another process): the
                // removal marked its files missing and took the walk's lease, so the walk wrote nothing after it.
                // Nothing failed, and the folder isn't asked for again.
                Err(ScanError::Removed { .. }) => {}
                // The walk's lease expired while it was held up (section 5): the folder is still in the library (a
                // removal ends a walk as `Removed`), so it is walked again after the busy retry, from its root. The
                // first losses say nothing; [`LOSSES_SAID`] in a row say so, kept as a condition of the folder.
                Err(error @ ScanError::LeaseLost { .. }) => {
                    if cx.inner.lease_lost(&path) >= LOSSES_SAID {
                        cx.inner.notify(Notice::ScanFailed {
                            root: path.clone(),
                            error: error_chain(&error),
                            walk: true,
                        });
                    }
                    // Its batches before the loss are in.
                    cx.inner.scanned();
                    cx.inner.walk_busy(path);
                }
                Err(ScanError::Failed { error, .. }) => {
                    cx.inner.notify(Notice::ScanFailed { root: path, error: error_chain(&error), walk: true });
                    // Its batches before the error are in.
                    cx.inner.scanned();
                }
            }
        }
    }
}

/// Analyzes what the scans found, from each file's header alone (`pg_import::import_while`): no hash, no pixel. The
/// duplicates are the Hashes pass's, after the thumbnails.
pub(crate) struct Import;

impl Pass<Cx<'_>> for Import {
    fn run(&mut self, cx: &mut Cx<'_>) -> Result<Step, Failed> {
        let volumes = cx.inner.volume_set();
        let opts = cx.inner.options(TunedPass::Import);
        let s = pg_import::import_while(cx.catalog, &opts, &volumes, &|| true, &cx.inner.ctl)?;
        if s.files > 0 {
            cx.inner.imported();
        }
        Ok(Step::Done)
    }
}

/// Makes the duplicate hashes the import stored without them, while duplicates detection was off (`pg_import::hashes`).
/// Its pass ends after a batch when duplicates detection is turned off; what is left stays owed (`hashes_owed`).
pub(crate) struct Hashes;

impl Pass<Cx<'_>> for Hashes {
    fn run(&mut self, cx: &mut Cx<'_>) -> Result<Step, Failed> {
        let features = cx.inner.shared.features.clone();
        if !features.get().duplicates {
            // Nothing is hashed, so nothing is unreadable: a notice from an earlier round is no longer true.
            cx.inner.files_read(FilePass::Import, 0, true);
            return Ok(Step::Done);
        }
        let volumes = cx.inner.volume_set();
        let s = pg_import::hashes::hash(
            cx.catalog,
            &cx.inner.options(TunedPass::Hashes),
            &volumes,
            &|| features.get().duplicates,
            &cx.inner.ctl,
        )?;
        // Its stored hashes are new pictures for the pairing pass, and the views list the duplicates again.
        if s.stored > 0 {
            cx.inner.imported();
        }
        // A file that couldn't be read is said, as the borders' are; one that didn't decode is an import failure, which
        // the import's ledger holds.
        cx.inner.files_read(FilePass::Import, s.unreadable, s.complete);
        Ok(Step::Done)
    }
}

/// Pairs what the import and the hashes left owed (`pg_import::pairs`): the pictures analyzed since the last pass, against
/// every picture hashed, of every lane. Skipped while duplicates detection is off: the owed pairs wait (`pairs_owed`) for
/// it to come back on. One lane's join at a time (`PairGate`): its join runs to the end once started, as a pass of seconds
/// to minutes, unless the lane is stopped, when the join is wanted again by the next lane's round.
pub(crate) struct Pairing;

impl Pass<Cx<'_>> for Pairing {
    fn run(&mut self, cx: &mut Cx<'_>) -> Result<Step, Failed> {
        if !cx.inner.shared.features.get().duplicates {
            return Ok(Step::Done);
        }
        let gate = &cx.inner.shared.pairing;
        if !gate.begin() {
            return Ok(Step::Done);
        }
        while gate.next() {
            let s = match pg_import::pairs::pair(cx.catalog, &cx.inner.base_options(), &cx.inner.ctl) {
                Ok(s) => s,
                Err(e) => {
                    gate.abandon();
                    return Err(e.into());
                }
            };
            // The duplicates the views list change with the pairs stored.
            if s.pairs > 0 || s.files > 0 {
                cx.inner.imported();
            }
            if cx.inner.ctl.stop_requested() {
                gate.abandon();
                break;
            }
        }
        Ok(Step::Done)
    }
}

/// Looks for the borders of what was imported. Its own pass, after the import: the import's decode is reduced, the
/// detector wants full size. Skipped while border detection is off, and its due files stay due (`borders_due`), so
/// turning it back on finds them; it ends after a batch when it is turned off.
pub(crate) struct Borders;

impl Pass<Cx<'_>> for Borders {
    fn run(&mut self, cx: &mut Cx<'_>) -> Result<Step, Failed> {
        let features = cx.inner.shared.features.clone();
        if !features.get().borders {
            // Nothing is looked for, so nothing is unreadable: a notice from an earlier pass is no longer true.
            cx.inner.files_read(FilePass::Borders, 0, true);
            return Ok(Step::Done);
        }
        let volumes = cx.inner.volume_set();
        let s = pg_borders::find_while(
            cx.catalog,
            &cx.inner.options(TunedPass::Borders),
            &volumes,
            &|| features.get().borders,
            &cx.inner.ctl,
        )?;
        // Only what went into the catalog changes what the app shows: files that couldn't be read stored nothing.
        if s.stored > 0 {
            cx.inner.imported();
        }
        cx.inner.files_read(FilePass::Borders, s.unreadable, s.complete);
        // The borders' ledger rows are written in the transaction of the borders they go with (`pg_borders::find`): a
        // failure to write them fails the batch, which is the round's failure, not a ledger's of its own.
        Ok(Step::Done)
    }
}

/// Makes the thumbnails still missing at the grid's current height. It runs before the duplicates and the borders: the
/// grid shows them first. It ends after a batch when the grid asks for another height; the next round makes that one.
pub(crate) struct Thumbs;

impl Pass<Cx<'_>> for Thumbs {
    fn run(&mut self, cx: &mut Cx<'_>) -> Result<Step, Failed> {
        // No height yet: the grid has not said which thumbnails it wants, so there is nothing to make.
        let Some(height) = cx.inner.shared.heights.get() else {
            return Ok(Step::Done);
        };
        if cx.inner.shared.thumbs.suspended() {
            // The cache is off (the Library panel): nothing is made or stored. Nothing is unreadable now, and a lane that
            // never runs this pass has no tally entry, so the notices of the pass are settled here: a stale "cache full" or
            // "unreadable files" would otherwise stay (they re-post when the cache is back on and the pass runs).
            cx.inner.files_read(FilePass::Thumbs, 0, true);
            cx.inner.cache_room();
            return Ok(Step::Done);
        }
        let heights = cx.inner.shared.heights.clone();
        let opts = cx.inner.options(TunedPass::Thumbs);
        let volumes = cx.inner.volume_set();
        let thumbs = &cx.inner.shared.thumbs;
        let ran = pg_thumbs::run_while(
            thumbs,
            &cx.inner.shared.catalog_path,
            height,
            &opts,
            &volumes,
            &|| heights.get() == Some(height) && !thumbs.suspended(),
            &pg_thumbs::new_maker,
            &cx.inner.ctl,
        );
        // What the pass did is reported however it ended (section 10, D6): its ledger outcome, its unreadable files.
        let (s, error) = match ran {
            Ok(s) => (s, None),
            Err(stopped) => {
                let pg_thumbs::Stopped { error, stats } = *stopped;
                (stats, Some(error))
            }
        };
        if s.cache_full {
            cx.inner.cache_full(height, s.made);
        } else if s.complete {
            cx.inner.cache_room();
        }
        cx.inner.files_read(FilePass::Thumbs, s.unreadable, s.complete);
        cx.inner.ledger_written(s.ledger);
        match error {
            // The cache was read: a failed read's notice no longer holds.
            None => {
                cx.inner.shared.notices.resolve(Trigger::Op(Op::CacheRead));
                Ok(Step::Done)
            }
            Some(error) => cache_failure(cx, error),
        }
    }
}

/// A thumbnail or placeholder pass's error, classified (section 10, D7). The cache's store and read failures are said with
/// their own states (`CacheNotStored`, `CacheNotRead`) and make the round a failed one for the backoff without a
/// `RoundFailed` (`Cx::said`); the passes after it still run (section 3). Every other error (the catalog, the maker's
/// start, the pipeline) is the round's.
fn cache_failure(cx: &mut Cx<'_>, error: pg_thumbs::Error) -> Result<Step, Failed> {
    match error {
        pg_thumbs::Error::CacheStore(e) => {
            cx.inner.notify(Notice::CacheNotStored { error: error_chain(&e) });
            *cx.said = true;
            Ok(Step::Done)
        }
        pg_thumbs::Error::CacheRead(e) => {
            cx.inner.notify(Notice::CacheNotRead { error: error_chain(&e) });
            *cx.said = true;
            Ok(Step::Done)
        }
        other => Err(Failed::new(pg_jobs::failure::Class::Other, other)),
    }
}

/// Makes the ThumbHash placeholders the thumbnails left owing (`pg_thumbs::placeholders`), after them. It is not tied to
/// the grid's height: a placeholder is read from a cached thumbnail or from the original, whatever the grid shows. The
/// thumbnail cache's setting (the Library panel) turns it off too: while the cache is off nothing is made, and the
/// placeholders already stored keep showing.
pub(crate) struct Placeholders;

impl Pass<Cx<'_>> for Placeholders {
    fn run(&mut self, cx: &mut Cx<'_>) -> Result<Step, Failed> {
        if cx.inner.shared.thumbs.suspended() {
            return Ok(Step::Done);
        }
        // Its failures (the files that don't decode) are the Thumbs ledger's rows, which the pass writes in its batches.
        let volumes = cx.inner.volume_set();
        let thumbs = &cx.inner.shared.thumbs;
        let opts = cx.inner.options(TunedPass::Placeholders);
        let ran = pg_thumbs::placeholders::run_while(
            cx.catalog,
            thumbs,
            &opts,
            &volumes,
            &|| !thumbs.suspended(),
            &pg_thumbs::new_maker,
            &cx.inner.ctl,
        );
        match ran {
            Ok(_) => Ok(Step::Done),
            Err(stopped) => cache_failure(cx, stopped.error),
        }
    }
}
