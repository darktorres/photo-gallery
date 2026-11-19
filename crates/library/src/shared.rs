//! What the job's lanes and its runner share: the catalog's path, the thumbnail cache, the cells the app sets, the notices
//! and the change bus, the tally of the lanes' notices, the pairing gate, and the job's counts. The cells, the generation
//! and the lanes' progress are public types of the runner (`runner` re-exports them); they live here, so that the lanes and
//! the coordinator name them without naming the runner, which names the coordinator.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use pg_jobs::NoticeSink;
use pg_jobs::events::{Bus, Change};
use pg_model::features::Features;
use pg_model::progress::Progress;
use pg_model::size::ThumbHeight;
use pg_thumbcache::ThumbCache;

use crate::pair_gate::PairGate;
use crate::tally::Tally;

/// The thumbnail height the grid asks for. It is held here, not given to the job at its start, so the height may be set
/// before the job starts (the window's scale arrives after the library is made). The thumbnail pass makes thumbnails only
/// once a height is set. A change wakes the job, and the thumbnail pass in progress ends after its batch (it watches the
/// height it runs for, `run`); a round is not stopped.
#[derive(Clone, Default)]
pub struct HeightCell(Arc<Mutex<HeightState>>);

/// The background features the owner has on (`Features`, from the preferences), held here so a change reaches the job
/// without a restart. A change wakes the job; it stops nothing. Each pass a feature gates reads it when it starts, and
/// ends after its batch when its feature is turned off (`still` in each pass).
#[derive(Clone)]
pub struct FeatureCell(Arc<Mutex<FeatureState>>);

struct FeatureState {
    features: Features,
    /// Run on a change, once a job listens (`FeatureCell::listen`).
    wake: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl FeatureCell {
    pub fn new(features: Features) -> FeatureCell {
        FeatureCell(Arc::new(Mutex::new(FeatureState { features, wake: None })))
    }

    /// The features, as they are now.
    pub fn get(&self) -> Features {
        self.state().features
    }

    /// Set the features. A change wakes the job, if one listens.
    pub fn set(&self, features: Features) {
        let wake = {
            let mut state = self.state();
            if state.features == features {
                return;
            }
            state.features = features;
            state.wake.clone()
        };
        if let Some(wake) = wake {
            wake();
        }
    }

    /// Run `wake` on each change from now on (the job's own, installed at its start).
    pub(crate) fn listen(&self, wake: impl Fn() + Send + Sync + 'static) {
        self.state().wake = Some(Arc::new(wake));
    }

    fn state(&self) -> MutexGuard<'_, FeatureState> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[derive(Default)]
struct HeightState {
    height: Option<ThumbHeight>,
    /// Run on a change, once a job listens (`HeightCell::listen`).
    wake: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl HeightCell {
    /// The height, once one has been set.
    pub fn get(&self) -> Option<ThumbHeight> {
        self.state().height
    }

    /// Set the height. A change to a different height wakes the job, if one listens.
    pub fn set(&self, height: ThumbHeight) {
        let wake = {
            let mut state = self.state();
            if state.height == Some(height) {
                return;
            }
            state.height = Some(height);
            state.wake.clone()
        };
        if let Some(wake) = wake {
            wake();
        }
    }

    /// Run `wake` on each change from now on (the job's own, installed at its start).
    pub(crate) fn listen(&self, wake: impl Fn() + Send + Sync + 'static) {
        self.state().wake = Some(Arc::new(wake));
    }

    fn state(&self) -> MutexGuard<'_, HeightState> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// What the job has done so far, for the app to refresh its views.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub struct Generation {
    /// Folder scans that changed the catalog: files new, moved, renamed, rewritten, back or gone (the folder tree
    /// and the lists changed). A scan that found the files as they were counts for nothing: the app would read its
    /// tree and its list again for no change.
    pub scans: u64,
    /// Import passes that analyzed something (sizes and placeholders changed).
    pub imports: u64,
}

/// What one lane of the job is doing: its drive's label for people (`E:`, empty when the drive can't be told), and its
/// progress.
#[derive(Clone, Debug, PartialEq)]
pub struct LaneProgress {
    /// The lane's number: its identity for `Runner::set_lane_paused` (numbers are not reused).
    pub number: usize,
    pub label: String,
    pub progress: Progress,
    /// The time since this lane last had a phase running: `Some(ZERO)` while one runs, `None` before its first.
    pub since_work: Option<Duration>,
    /// Whether this lane is paused itself (not the run's global pause).
    pub paused: bool,
}

/// What the lanes share: the catalog's path, the thumbnail cache, the cells, the notices and the change bus, the tally of
/// the lanes' notices, the pairing gate, and the job's counts (what the app reads to refresh its views).
pub(crate) struct Shared {
    pub(crate) catalog_path: PathBuf,
    pub(crate) thumbs: &'static ThumbCache,
    pub(crate) heights: HeightCell,
    pub(crate) features: FeatureCell,
    pub(crate) notices: Arc<dyn NoticeSink>,
    pub(crate) changes: Arc<Bus<Change>>,
    pub(crate) tally: Tally,
    pub(crate) pairing: PairGate,
    scans_done: AtomicU64,
    imports_done: AtomicU64,
    pub(crate) rounds: AtomicU64,
}

impl Shared {
    pub(crate) fn new(
        catalog_path: PathBuf,
        thumbs: &'static ThumbCache,
        heights: HeightCell,
        features: FeatureCell,
        notices: Arc<dyn NoticeSink>,
        changes: Arc<Bus<Change>>,
    ) -> Shared {
        let tally = Tally::new(notices.clone());
        Shared {
            catalog_path,
            thumbs,
            heights,
            features,
            notices,
            changes,
            tally,
            pairing: PairGate::default(),
            scans_done: AtomicU64::new(0),
            imports_done: AtomicU64::new(0),
            rounds: AtomicU64::new(0),
        }
    }

    /// A walk changed the catalog.
    pub(crate) fn scanned(&self) {
        self.scans_done.fetch_add(1, Ordering::Relaxed);
        self.changes.publish(Change::Scanned);
    }

    /// A pass changed what the views show.
    pub(crate) fn imported(&self) {
        self.imports_done.fetch_add(1, Ordering::Relaxed);
        self.changes.publish(Change::Imported);
    }

    /// A lane's round ended: counted, and published for the owner's views.
    pub(crate) fn round_ended(&self) {
        self.rounds.fetch_add(1, Ordering::Relaxed);
        self.changes.publish(Change::Round);
    }

    pub(crate) fn generation(&self) -> Generation {
        Generation {
            scans: self.scans_done.load(Ordering::Relaxed),
            imports: self.imports_done.load(Ordering::Relaxed),
        }
    }
}
