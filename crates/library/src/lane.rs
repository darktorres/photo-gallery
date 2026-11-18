//! One lane of the background job: a thread that runs the round of passes over its volumes, one lane per drive (`lanes`).
//! The coordinator (`coordinator`) makes the lanes, gives each its volumes and its scans, and retires them; a lane only
//! does its round. The round is the passes in order (`passes`): the folders asked for are walked first, then the files are
//! imported, thumbnailed, placed, hashed, paired, and their borders looked for.
//!
//! A lane stops its round at a request (`Control::request_stop`), and waits while it is held (`Control::hold`): a crop or
//! a cut that changes this lane's volumes holds the lane, and waits until the round in progress has ended, before it works.

use std::collections::VecDeque;
use std::panic::AssertUnwindSafe;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use anyhow::Result;
use pg_catalog::{Catalog, VolumeSet};
use pg_jobs::control::Control;
use pg_jobs::governor::Budget;
use pg_jobs::panics;
use pg_jobs::pipeline::Options;
use pg_jobs::schedule::{self, Step};
use pg_jobs::service::{Waker, Woke};
use pg_model::notice::{ERROR_CAP, FilePass, Notice, tidy};
use pg_model::rules::Path as RulePath;
use pg_model::size::ThumbHeight;
use pg_platform::native::PATH_RULES;
use pg_platform::priority::Priority;
use pg_thumbs::LedgerOutcome;

use crate::passes::{self, Cx};
use crate::runner::Shared;

/// How long a walk refused by another walk's lease (an overlapping folder, in this process or another) waits before it
/// is tried again, with nothing else to wake the lane: a lease ends with its walk, or expires in
/// `pg_catalog::SCAN_LEASE_SECS`.
pub(crate) const BUSY_RETRY: Duration = Duration::from_secs(5);

/// A lane's state, shared by its thread and by whoever routes to it (the coordinator, the runner's requests).
pub(crate) struct Inner {
    pub(crate) shared: Arc<Shared>,
    /// The lane's number, as the tally knows it.
    pub(crate) number: usize,
    /// This lane's control: a child of the run's root (`Control::lane`), with its own stop, pause and holds.
    pub(crate) ctl: Arc<Control>,
    /// The lane's share of the run's file-buffer budget (the governor sets it: `pg_jobs::governor`).
    budget: Arc<Budget>,
    /// The lane's drive for people (`E:`), empty when it can't be told (see [`Inner::label`]).
    label: Mutex<String>,
    /// The volumes this lane's passes read.
    volumes: Mutex<VolumeSet>,
    /// The folders to walk, in order: the coordinator routes each request here.
    scans: Mutex<VecDeque<RulePath>>,
    /// The folder a scan is walking now (`Runner::stop_scans_under`).
    pub(crate) scanning: Mutex<Option<RulePath>>,
    /// The scans refused because another walk of an overlapping folder held its lease: back in the queue at the next
    /// round, which comes by [`BUSY_RETRY`] at the latest.
    busy: Mutex<Vec<RulePath>>,
    /// Whether a round is running (`active`, with `idle` told when it ends): a holder waits for it to end.
    active: Mutex<bool>,
    idle: Condvar,
}

/// Why a round failed: an error, or a panic (which the lane reports, and goes on).
enum Fault {
    Failed(String),
    Panicked(String),
}

impl Inner {
    pub(crate) fn new(shared: Arc<Shared>, number: usize, ctl: Arc<Control>, budget: Arc<Budget>) -> Inner {
        Inner {
            shared,
            number,
            ctl,
            budget,
            label: Mutex::new(String::new()),
            volumes: Mutex::new(VolumeSet::All),
            scans: Mutex::new(VecDeque::new()),
            scanning: Mutex::new(None),
            busy: Mutex::new(Vec::new()),
            active: Mutex::new(false),
            idle: Condvar::new(),
        }
    }

    /// Pass options for this lane at `priority`: the lane's budget is the pass's, so the governor's share holds.
    pub(crate) fn options(&self, priority: Priority) -> Options {
        Options { budget: Some(self.budget.clone()), ..Options::new(priority) }
    }

    /// Whether a round is running now.
    pub(crate) fn is_active(&self) -> bool {
        *self.active.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The volumes this lane's passes read.
    pub(crate) fn volume_set(&self) -> VolumeSet {
        self.volumes.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Gives the lane its volumes; a round in progress keeps the ones it began with.
    pub(crate) fn set_volumes(&self, volumes: VolumeSet) {
        *self.volumes.lock().unwrap_or_else(PoisonError::into_inner) = volumes;
    }

    /// The folders to walk. A holder that panicked left it whole (a request is pushed or popped, or not).
    pub(crate) fn scans(&self) -> MutexGuard<'_, VecDeque<RulePath>> {
        self.scans.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The scans refused by another walk's lease, waiting for their retry.
    pub(crate) fn busy(&self) -> MutexGuard<'_, Vec<RulePath>> {
        self.busy.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The walk of `path` was refused by another walk's lease: it is tried again later ([`BUSY_RETRY`]).
    pub(crate) fn walk_busy(&self, path: RulePath) {
        let mut busy = self.busy();
        if !busy.contains(&path) {
            busy.push(path);
        }
    }

    /// The scans refused by a lease go back to the end of the queue, for this round to try again.
    pub(crate) fn requeue_busy(&self) {
        let busy = std::mem::take(&mut *self.busy());
        let mut scans = self.scans();
        for path in busy {
            if !scans.contains(&path) {
                scans.push_back(path);
            }
        }
    }

    /// Everything the lane holds, given up when it is retired: its queued and refused scans, for the coordinator to route.
    pub(crate) fn take_scans(&self) -> Vec<RulePath> {
        let mut all: Vec<RulePath> = self.scans().drain(..).collect();
        all.append(&mut self.busy());
        all
    }

    /// Whether the lane is running a round, and waits until it isn't, for at most as long as `stop` is not asked for.
    /// `false` when `stop` was asked for first. A held lane's round ends after its batch, so this is brief.
    pub(crate) fn wait_idle(&self, stop: &Control) -> bool {
        let mut active = self.active.lock().unwrap_or_else(PoisonError::into_inner);
        while *active {
            if stop.stop_requested() {
                return false;
            }
            active =
                self.idle.wait_timeout(active, Duration::from_millis(50)).unwrap_or_else(PoisonError::into_inner).0;
        }
        true
    }

    fn set_active(&self, running: bool) {
        let mut active = self.active.lock().unwrap_or_else(PoisonError::into_inner);
        *active = running;
        if !running {
            self.idle.notify_all();
        }
    }

    pub(crate) fn notify(&self, notice: Notice) {
        self.shared.notices.post(notice);
    }

    /// A pass couldn't read `unreadable` of its files, and `complete` says whether it read the rest (`Tally::files_read`).
    pub(crate) fn files_read(&self, pass: FilePass, unreadable: usize, complete: bool) {
        let drive = self.label();
        self.shared.tally.files_read(self.number, &drive, pass, unreadable, complete);
    }

    /// The lane's drive for people (`E:`), set by the coordinator when it plans the lanes; empty when it can't be told.
    pub(crate) fn label(&self) -> String {
        self.label.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Sets the lane's drive label (see [`Inner::label`]).
    pub(crate) fn set_label(&self, label: String) {
        *self.label.lock().unwrap_or_else(PoisonError::into_inner) = label;
    }

    /// The thumbnail cache is full at `height`, after `made` thumbnails (`Tally::cache_full`).
    pub(crate) fn cache_full(&self, height: ThumbHeight, made: usize) {
        self.shared.tally.cache_full(self.number, height, made);
    }

    /// The thumbnails pass ended with the cache not full (`Tally::cache_room`).
    pub(crate) fn cache_room(&self) {
        self.shared.tally.cache_room(self.number);
    }

    /// The thumbnails' failure ledger write came to `outcome` (`Tally::ledger`).
    pub(crate) fn ledger_written(&self, outcome: LedgerOutcome) {
        self.shared.tally.ledger(self.number, outcome);
    }

    pub(crate) fn scanned(&self) {
        self.shared.scanned();
    }

    pub(crate) fn imported(&self) {
        self.shared.imported();
    }

    /// The lane's thread: rounds, one after another, each started by a request, a wake or the busy retry.
    pub(crate) fn run(&self, waker: &Waker<'_>) {
        // One connection for every round; after a round that failed, a new one.
        let mut catalog: Option<Catalog> = None;
        // A round stopped by a new request starts the next one at once: the request is already queued.
        let mut again = false;
        loop {
            if !again {
                let idle = (!self.busy().is_empty()).then_some(BUSY_RETRY);
                if waker.wait(idle) == Woke::Quit {
                    return;
                }
            }
            again = false;
            // A held lane waits for its hold to end: a crop or a cut works on its volumes meanwhile. A request that stopped
            // the lane while it waited is cleared below, with the requests' stops: each request queued its work.
            self.ctl.wait_while_held(&|| waker.quitting());
            // The stop the last request set is cleared here, then the quit is looked for: a quit asked after this check
            // comes with a stop after the clear, so the round stops.
            self.ctl.clear_stop();
            if waker.quitting() {
                return;
            }
            self.requeue_busy();
            self.set_active(true);
            let outcome = self.round_caught(&mut catalog);
            self.set_active(false);
            match outcome {
                Ok(step) => {
                    // A round that ran is the end of a failed one's notice.
                    self.shared.tally.round(self.number, None);
                    again = matches!(step, Step::Preempted);
                }
                Err(Fault::Failed(error)) => self.shared.tally.round(self.number, Some(error)),
                Err(Fault::Panicked(message)) => {
                    self.notify(Notice::RoundPanicked { message: tidy(&message, ERROR_CAP) });
                }
            }
            self.shared.round_ended();
        }
    }

    /// A round, with a panic in it caught: the lane reports it (the inbox, and the log through the panic hook) and goes on
    /// with the next request.
    fn round_caught(&self, catalog: &mut Option<Catalog>) -> std::result::Result<Step, Fault> {
        match panics::catch(AssertUnwindSafe(|| self.round(catalog))) {
            Ok(result) => result.map_err(|e| Fault::Failed(tidy(&format!("{e:#}"), ERROR_CAP))),
            Err(panic) => {
                // The connection a panicking pass held is not trusted: the next round opens a new one.
                *catalog = None;
                Err(Fault::Panicked(panics::message(&*panic).to_string()))
            }
        }
    }

    /// One round: the passes in order, on the connection the lane keeps between rounds.
    pub(crate) fn round(&self, conn: &mut Option<Catalog>) -> Result<Step> {
        // The round's span in the perf log: the passes it runs are its children, and the catalog's open is timed apart.
        let _round = pg_perf::span("round");
        let open = pg_perf::span("round.open");
        let mut catalog = match conn.take() {
            Some(c) => c,
            None => Catalog::open(&self.shared.catalog_path, PATH_RULES)?,
        };
        drop(open);
        let step = {
            let mut cx = Cx { inner: self, catalog: &mut catalog };
            schedule::round(
                &mut [
                    &mut passes::Scans,
                    &mut passes::Import,
                    &mut passes::Thumbs,
                    &mut passes::Placeholders,
                    &mut passes::Hashes,
                    &mut passes::Pairing,
                    &mut passes::Borders,
                ],
                &mut cx,
                |cx| cx.inner.ctl.stop_requested(),
            )?
        };
        *conn = Some(catalog);
        Ok(step)
    }
}
