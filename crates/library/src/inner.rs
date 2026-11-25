//! A lane's state: the data its thread, its passes and the coordinator share. The thread that runs the rounds is `lane`,
//! the passes are `passes`, and the coordinator routes requests to a lane through this state. A leaf of the lane's
//! layers: it names nothing above it, so the passes can take `&Inner` without the thread that calls them.

use std::collections::VecDeque;
#[cfg(test)]
use std::num::NonZeroUsize;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use pg_catalog::VolumeSet;
use pg_jobs::control::Control;
use pg_jobs::governor::Budget;
use pg_jobs::pipeline::{Options, SizeRefused};
use pg_model::notice::{FilePass, Notice};
use pg_model::rules::Path as RulePath;
use pg_model::size::ThumbHeight;
use pg_platform::priority::Priority;
use pg_thumbs::LedgerOutcome;

use crate::shared::{Pins, Shared};

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
    /// round, which comes by the busy retry (`lane::BUSY_RETRY`) at the latest.
    busy: Mutex<Vec<RulePath>>,
    /// Whether a round is running (`active`, with `idle` told when it ends): a holder waits for it to end.
    active: Mutex<bool>,
    idle: Condvar,
}

/// The pass options a run's lanes give their passes, below normal (the one background priority): the defaults, or the
/// counts a run pinned (`Pins`), with the slots and the budget that follow from them as they do by default. Refused when
/// the pinned counts are out of range (`Options::sized`): the runner does not start with them.
pub(crate) fn pinned_options(pins: Pins) -> Result<Options, SizeRefused> {
    let default = Options::new(Priority::BelowNormal);
    let readers = pins.readers.unwrap_or(default.readers);
    let workers = pins.workers.unwrap_or(default.workers);
    Options::sized(readers, workers, Priority::BelowNormal)
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

    /// Pass options for this lane, below normal: the run's (`Shared::options`, with the counts a pinned run took), and the
    /// lane's budget as the pass's, so the governor's share holds.
    pub(crate) fn options(&self) -> Options {
        Options { budget: Some(self.budget.clone()), ..self.shared.options.clone() }
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

    /// The walk of `path` was refused by another walk's lease: it is tried again later (the busy retry).
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

    /// Marks a round as running (`running`) or ended; the end wakes the holders in `wait_idle`.
    pub(crate) fn set_active(&self, running: bool) {
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
}

#[cfg(test)]
mod pin_tests {
    use super::*;

    fn n(count: usize) -> NonZeroUsize {
        NonZeroUsize::new(count).unwrap()
    }

    #[test]
    fn unpinned_options_are_the_defaults_below_normal() {
        let opts = pinned_options(Pins::default()).unwrap();
        let default = Options::new(Priority::BelowNormal);
        assert_eq!((opts.readers, opts.workers), (default.readers, default.workers));
        assert_eq!(opts.slots, default.slots);
        assert_eq!(opts.buffer_bytes, default.buffer_bytes);
        assert_eq!(opts.priority, Priority::BelowNormal);
        assert!(opts.budget.is_none());
    }

    #[test]
    fn pinned_counts_replace_the_defaults_and_the_slots_and_budget_follow() {
        let pins = Pins { readers: Some(n(3)), workers: Some(n(2)) };
        let opts = pinned_options(pins).unwrap();
        assert_eq!((opts.readers, opts.workers), (n(3), n(2)));
        assert_eq!(Some(opts.slots), Options::slots(n(3), n(2)));
        assert_eq!(Some(opts.buffer_bytes), Options::budget(n(3), n(2)));
    }

    #[test]
    fn pinned_counts_out_of_range_are_refused() {
        let pins = Pins { readers: None, workers: Some(n(pg_jobs::pipeline::MAX_SLOTS)) };
        assert_eq!(pinned_options(pins).err(), Some(SizeRefused::TooManySlots));
    }

    #[test]
    fn a_lanes_options_are_the_runs_with_the_lanes_budget() {
        let budget = Arc::new(Budget::new(1 << 20));
        let run = pinned_options(Pins { readers: Some(n(3)), workers: Some(n(2)) }).unwrap();
        let lane = Options { budget: Some(budget.clone()), ..run.clone() };
        assert_eq!((lane.readers, lane.workers, lane.slots), (run.readers, run.workers, run.slots));
        assert!(lane.budget.as_ref().is_some_and(|shared| Arc::ptr_eq(shared, &budget)), "the lane's budget is kept");
    }
}
