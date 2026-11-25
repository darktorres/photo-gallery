//! A lane's state: the data its thread, its passes and the coordinator share. The thread that runs the rounds is `lane`,
//! the passes are `passes`, and the coordinator routes requests to a lane through this state. A leaf of the lane's
//! layers: it names nothing above it, so the passes can take `&Inner` without the thread that calls them.

use std::collections::VecDeque;
#[cfg(test)]
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use pg_catalog::VolumeSet;
use pg_jobs::control::Control;
use pg_jobs::pipeline::{self, Options, Readers, SizeRefused, Tuning};
use pg_jobs::tuner::{Decision, MAX_READERS, TunerConfig};
use pg_model::notice::{FilePass, Notice};
use pg_model::rules::Path as RulePath;
use pg_model::size::ThumbHeight;
use pg_platform::drive::Drive;
use pg_platform::priority::Priority;
use pg_thumbs::LedgerOutcome;

use crate::shared::{Pins, Shared};
use crate::tuning::{Binding, LaneTuner, TunedPass, now_secs};

/// A lane's state, shared by its thread and by whoever routes to it (the coordinator, the runner's requests).
pub(crate) struct Inner {
    pub(crate) shared: Arc<Shared>,
    /// The lane's number, as the tally knows it.
    pub(crate) number: usize,
    /// This lane's control: a child of the run's root (`Control::lane`), with its own stop, pause and holds.
    pub(crate) ctl: Arc<Control>,
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
    /// The lane's tuners, one per pass kind, and its drive (`tuning`).
    tuning: Mutex<LaneTuner>,
    /// The reads in flight its last tuned pass let through (0: none yet), for the lane's progress.
    readers_now: Arc<AtomicUsize>,
}

/// The pass options a run's lanes give their passes, below normal (the one background priority): the defaults, or the
/// counts a run pinned (`Pins`), with the slots that follow from them as they do by default. Unpinned, the
/// readers are sized for the most a tuned pass takes ([`MAX_READERS`]); each lane hands its passes its tuners
/// ([`Inner::options`]). Refused when the pinned counts are out of range (`Options::sized`): the runner does not start
/// with them.
pub(crate) fn pinned_options(pins: Pins) -> Result<Options, SizeRefused> {
    let readers = pins.readers.unwrap_or(MAX_READERS);
    let workers = pins.workers.unwrap_or_else(pipeline::default_workers);
    Options::sized(Readers::Fixed(readers), workers, Priority::BelowNormal)
}

impl Inner {
    pub(crate) fn new(shared: Arc<Shared>, number: usize, ctl: Arc<Control>) -> Inner {
        Inner {
            shared,
            number,
            ctl,
            label: Mutex::new(String::new()),
            volumes: Mutex::new(VolumeSet::All),
            scans: Mutex::new(VecDeque::new()),
            scanning: Mutex::new(None),
            busy: Mutex::new(Vec::new()),
            active: Mutex::new(false),
            idle: Condvar::new(),
            tuning: Mutex::new(LaneTuner::new(TunerConfig::default())),
            readers_now: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// The run's pass options, with no tuner: for a pass that reads no file through the readers (the pairing).
    pub(crate) fn base_options(&self) -> Options {
        self.shared.options.clone()
    }

    /// Pass options for this lane's `pass`, below normal: [`Inner::base_options`], with the lane's tuner for the pass
    /// kind, unless the run pinned its readers.
    pub(crate) fn options(&self, pass: TunedPass) -> Options {
        let mut opts = self.base_options();
        if self.shared.pinned {
            return opts;
        }
        let tuner = self.tuning().tuner(pass, self.shared.profiles.as_deref(), now_secs());
        let first = tuner.lock().unwrap_or_else(PoisonError::into_inner).target().get();
        self.readers_now.store(first, Ordering::Relaxed);
        let readers_now = self.readers_now.clone();
        let log = self.shared.on_tuning.clone();
        let number = self.number;
        let drive = self.label();
        let on_change = move |decision: Decision| {
            readers_now.store(decision.target.get(), Ordering::Relaxed);
            if let Some(log) = &log {
                log(&format!(
                    "lane {number} ({drive}) {}: {} readers ({})",
                    pass.name(),
                    decision.target,
                    decision.reason.name()
                ));
            }
        };
        opts.readers = Readers::Auto(Tuning { max: MAX_READERS, tuner, on_change: Some(Arc::new(on_change)) });
        opts
    }

    fn tuning(&self) -> MutexGuard<'_, LaneTuner> {
        self.tuning.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Binds the lane's tuners to `drive` (the coordinator's probe of its roots): a different drive starts them again.
    pub(crate) fn bind_drive(&self, drive: Option<&Drive>) {
        self.tuning().bind(drive.map(Binding::of));
    }

    /// The reads in flight the lane's last tuned pass let through, if one ran.
    pub(crate) fn readers_now(&self) -> Option<usize> {
        Some(self.readers_now.load(Ordering::Relaxed)).filter(|&n| n > 0)
    }

    /// Keeps what the lane learned of its drive in the run's store, if it has one and the run is not pinned: when the
    /// lane goes idle, and when it is retired. A failed write is not the lane's failure: the profile is a hint.
    pub(crate) fn save_profiles(&self) {
        if self.shared.pinned {
            return;
        }
        if let Some(store) = &self.shared.profiles {
            let _ = self.tuning().write(store.as_ref(), now_secs());
        }
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
    fn unpinned_options_are_sized_for_a_tuned_pass_below_normal() {
        let opts = pinned_options(Pins::default()).unwrap();
        let workers = pipeline::default_workers();
        assert_eq!((opts.readers.max(), opts.workers), (MAX_READERS, workers));
        assert_eq!(Some(opts.slots), Options::slots(MAX_READERS, workers));
        assert_eq!(opts.priority, Priority::BelowNormal);
    }

    #[test]
    fn pinned_counts_replace_the_defaults_and_the_slots_follow() {
        let pins = Pins { readers: Some(n(3)), workers: Some(n(2)) };
        let opts = pinned_options(pins).unwrap();
        assert_eq!((opts.readers.max(), opts.workers), (n(3), n(2)));
        assert_eq!(Some(opts.slots), Options::slots(n(3), n(2)));
    }

    #[test]
    fn pinned_counts_out_of_range_are_refused() {
        let pins = Pins { readers: None, workers: Some(n(pg_jobs::pipeline::MAX_SLOTS)) };
        assert_eq!(pinned_options(pins).err(), Some(SizeRefused::TooManySlots));
    }
}
