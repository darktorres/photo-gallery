//! The passes of a lane's round, in order: the folders asked for are walked (`Scans`), the pictures they hold are imported,
//! their thumbnails and placeholders made, their duplicates hashed and paired, and the borders looked for. Each pass works
//! over the lane's volumes (`Inner::volume_set`), so the lanes of two drives run these passes at once, and the lanes of
//! one drive run them in turn.

use std::sync::PoisonError;

use anyhow::Result;
use pg_catalog::Catalog;
use pg_jobs::schedule::{Pass, Step};
use pg_model::ids::FileId;
use pg_model::notice::{ERROR_CAP, FileFailure, FilePass, Key, Notice, tidy};
use pg_model::rules::Path as RulePath;
use pg_platform::priority::Priority;
use pg_scan::ScanError;

use crate::inner::Inner;
use crate::scans;
use crate::shared::Shared;

/// A lane's pass context: the lane, and its round's connection.
pub(crate) struct Cx<'a> {
    pub(crate) inner: &'a Inner,
    pub(crate) catalog: &'a mut Catalog,
}

/// The notice of a crop job's file failure: its error capped as a notice holds it.
pub(crate) fn file_failure(failure: FileFailure) -> FileFailure {
    FileFailure { error: tidy(&failure.error, ERROR_CAP), ..failure }
}

/// A count as a notice holds it.
pub(crate) fn count(n: usize) -> u64 {
    u64::try_from(n).unwrap_or(u64::MAX)
}

/// An error's text as a notice holds it: its causes after it, on one line, capped.
pub(crate) fn error_text(error: &anyhow::Error) -> String {
    tidy(&format!("{error:#}"), ERROR_CAP)
}

/// Originals were cut in place: their thumbnails are forgotten (they show the uncut pixels), and the import that follows
/// analyzes them again. The app is told the files changed.
pub(crate) fn cut_done(shared: &Shared, cut: &[FileId]) {
    if cut.is_empty() {
        return;
    }
    for (_, e) in scans::forget_replaced(shared.thumbs, cut) {
        shared.notices.post(Notice::CutFailed { error: error_text(&e) });
    }
    shared.imported();
}

/// Walks the folders asked for, skipping those no longer in the library. A scan stopped early goes back to the front.
pub(crate) struct Scans;

impl Pass<Cx<'_>> for Scans {
    fn run(&mut self, cx: &mut Cx<'_>) -> Result<Step> {
        let shared = &cx.inner.shared;
        let rules = cx.catalog.rules();
        let library: Vec<RulePath> = cx.catalog.roots()?.into_iter().map(|r| r.path).collect();
        loop {
            let Some(path) = cx.inner.scans().pop_front() else {
                return Ok(Step::Done);
            };
            // A folder asked for that is no longer in the library is skipped.
            if !library.iter().any(|l| rules.is_under(&path, l)) {
                continue;
            }
            *cx.inner.scanning.lock().unwrap_or_else(PoisonError::into_inner) = Some(path.clone());
            let scanned = pg_scan::scan(cx.catalog, &path, &cx.inner.ctl);
            *cx.inner.scanning.lock().unwrap_or_else(PoisonError::into_inner) = None;
            // What a scan leaves to the runner after the scan's phase has closed: the replaced thumbnails, the temp files, the
            // notices.
            let _after = pg_perf::span("scan.after");
            match scanned {
                Ok(s) => {
                    // A scan stopped early has committed what it recorded so far.
                    if s.changed > 0 || s.missing > 0 || s.pairing.rebound > 0 || s.pairing.replaced > 0 {
                        cx.inner.scanned();
                    }
                    // What the pairing replaced loses its thumbnails and the temp files of its queued writes.
                    // Both come after the catalog's commit, which the scan made.
                    for (_, e) in scans::forget_replaced(shared.thumbs, &s.pairing.replaced_ids) {
                        cx.inner.notify(Notice::ScanFailed { root: path.clone(), error: error_text(&e) });
                    }
                    for (temp, e) in pg_writeback::discard_temps(rules, &s.pairing.temps) {
                        cx.inner.notify(Notice::ScanFailed {
                            root: path.clone(),
                            error: tidy(&format!("can't delete {}: {e}", rules.display(&temp)), ERROR_CAP),
                        });
                    }
                    let found = scans::scan_notices(&path, &s);
                    // What the walk did is said whatever folder it walked, and stopped or not: the pairing's binds and
                    // replacements are in, and the walk that resumes doesn't do them again.
                    for notice in found.events {
                        cx.inner.notify(notice);
                    }
                    if !s.complete {
                        cx.inner.scans().push_front(path);
                        return Ok(Step::Preempted);
                    }
                    // A complete walk of a whole library folder is what holds for it now: its kept notes (the folder's
                    // conditions) are replaced by the ones it found. A walk of a folder under it looked at part of the
                    // folder only, so it changes none of them. The folders it couldn't read are the folder ledger's,
                    // which the walk itself recorded and swept.
                    if library.contains(&path) {
                        shared.notices.replace(Key::all("scan-note", Some(&path)).to_buf(), found.conditions);
                    }
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
                Err(ScanError::Failed(e)) => {
                    cx.inner.notify(Notice::ScanFailed { root: path, error: error_text(&e) });
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
    fn run(&mut self, cx: &mut Cx<'_>) -> Result<Step> {
        let volumes = cx.inner.volume_set();
        let s = pg_import::import_while(
            cx.catalog,
            &cx.inner.options(Priority::BelowNormal),
            false,
            &volumes,
            &|| true,
            &cx.inner.ctl,
        )?;
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
    fn run(&mut self, cx: &mut Cx<'_>) -> Result<Step> {
        let features = cx.inner.shared.features.clone();
        if !features.get().duplicates {
            return Ok(Step::Done);
        }
        let volumes = cx.inner.volume_set();
        let s = pg_import::hashes::hash(
            cx.catalog,
            &cx.inner.options(Priority::BelowNormal),
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
    fn run(&mut self, cx: &mut Cx<'_>) -> Result<Step> {
        if !cx.inner.shared.features.get().duplicates {
            return Ok(Step::Done);
        }
        let gate = &cx.inner.shared.pairing;
        if !gate.begin() {
            return Ok(Step::Done);
        }
        while gate.next() {
            let s = match pg_import::pairs::pair(cx.catalog, &cx.inner.options(Priority::BelowNormal), &cx.inner.ctl) {
                Ok(s) => s,
                Err(e) => {
                    gate.abandon();
                    return Err(e);
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
    fn run(&mut self, cx: &mut Cx<'_>) -> Result<Step> {
        let features = cx.inner.shared.features.clone();
        if !features.get().borders {
            // Nothing is looked for, so nothing is unreadable: a notice from an earlier pass is no longer true.
            cx.inner.files_read(FilePass::Borders, 0, true);
            return Ok(Step::Done);
        }
        let volumes = cx.inner.volume_set();
        let s = pg_borders::find_while(
            cx.catalog,
            &cx.inner.options(Priority::BelowNormal),
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
    fn run(&mut self, cx: &mut Cx<'_>) -> Result<Step> {
        // No height yet: the grid has not said which thumbnails it wants, so there is nothing to make.
        let Some(height) = cx.inner.shared.heights.get() else {
            return Ok(Step::Done);
        };
        let heights = cx.inner.shared.heights.clone();
        let opts = cx.inner.options(Priority::Idle);
        let volumes = cx.inner.volume_set();
        let s = match pg_thumbs::run_while(
            cx.inner.shared.thumbs,
            &cx.inner.shared.catalog_path,
            height,
            &opts,
            &volumes,
            &|| heights.get() == Some(height),
            &cx.inner.ctl,
        ) {
            Ok(s) => s,
            // A pass that fails is said, and the round goes on to the duplicates and the borders, which are after it.
            Err(e) => {
                cx.inner.notify(Notice::CacheNotStored { error: error_text(&e) });
                return Ok(Step::Done);
            }
        };
        if s.cache_full {
            cx.inner.cache_full(height, s.made);
        } else if s.complete {
            cx.inner.cache_room();
        }
        cx.inner.files_read(FilePass::Thumbs, s.unreadable, s.complete);
        cx.inner.ledger_written(s.ledger);
        Ok(Step::Done)
    }
}

/// Makes the ThumbHash placeholders the thumbnails left owing (`pg_thumbs::placeholders`), after them. It is not tied to
/// the grid's height: a placeholder is read from a cached thumbnail or from the original, whatever the grid shows.
pub(crate) struct Placeholders;

impl Pass<Cx<'_>> for Placeholders {
    fn run(&mut self, cx: &mut Cx<'_>) -> Result<Step> {
        // Its failures (the files that don't decode) are the Thumbs ledger's rows, which the pass writes in its batches.
        let volumes = cx.inner.volume_set();
        pg_thumbs::placeholders::run_while(
            cx.catalog,
            cx.inner.shared.thumbs,
            &cx.inner.options(Priority::Idle),
            &volumes,
            &|| true,
            &cx.inner.ctl,
        )?;
        Ok(Step::Done)
    }
}
