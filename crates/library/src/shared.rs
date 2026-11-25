//! What the job's lanes and its runner share: the catalog's path, the thumbnail cache, the cells the app sets, the notices
//! and the change bus, the tally of the lanes' notices, the pairing gate, and the job's counts. The cells, the generation
//! and the lanes' progress are public types of the runner (`runner` re-exports them); they live here, so that the lanes and
//! the coordinator name them without naming the runner, which names the coordinator.

use std::collections::BTreeMap;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use pg_catalog::VolumeSet;
use pg_jobs::NoticeSink;
use pg_jobs::events::Bus;
use pg_model::features::Features;
use pg_model::job::Change;
use pg_model::removal::Volumes;
use pg_model::size::ThumbHeight;
use pg_thumbcache::ThumbCache;

use crate::mask::{PassCell, PassMask};
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
    /// A cell holding `features`, with no waiter yet.
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

/// The volumes a removal has parked (section 1, D5 and D6). A lane whose volumes include one of them waits at the start of
/// its round, until the removal releases them. Each removal parks under its folder's display path, and releases that key.
#[derive(Default)]
pub(crate) struct Parks {
    parked: Mutex<BTreeMap<String, Volumes>>,
    changed: Condvar,
}

/// Whether a lane on `set` is parked by `parked`: every lane is, when `parked.all` is set, else a lane on one of its volumes.
fn blocks(parked: &Volumes, set: &VolumeSet) -> bool {
    parked.all || parked.volumes.iter().any(|&volume| set.contains(volume))
}

impl Parks {
    /// Parks the lanes of `volumes` under `key`. Parking again under the same key replaces the volumes.
    pub(crate) fn park(&self, key: &str, volumes: Volumes) {
        self.parked.lock().unwrap_or_else(PoisonError::into_inner).insert(key.to_owned(), volumes);
        self.changed.notify_all();
    }

    /// Releases `key`'s parks: the lanes waiting for them may start their rounds.
    pub(crate) fn release(&self, key: &str) {
        self.parked.lock().unwrap_or_else(PoisonError::into_inner).remove(key);
        self.changed.notify_all();
    }

    /// The volumes parked under `key`, if any (the coordinator's own parks, for the lanes it parks itself).
    #[cfg(test)]
    pub(crate) fn parked(&self, key: &str) -> Option<Volumes> {
        self.parked.lock().unwrap_or_else(PoisonError::into_inner).get(key).cloned()
    }

    /// Waits while a lane on `set` is parked, for as long as `quit` is not asked. `false` when `quit` came first.
    pub(crate) fn wait_clear(&self, set: &VolumeSet, quit: &dyn Fn() -> bool) -> bool {
        let mut parked = self.parked.lock().unwrap_or_else(PoisonError::into_inner);
        while parked.values().any(|volumes| blocks(volumes, set)) {
            if quit() {
                return false;
            }
            parked =
                self.changed.wait_timeout(parked, Duration::from_millis(50)).unwrap_or_else(PoisonError::into_inner).0;
        }
        true
    }
}

/// A test's hooks into the job, at the two points a test holds it at: the start of a lane's round (inside the round, so a
/// park waits for the round, as it waits for a pass), and a removal's hide, before its park is taken. A hook runs without
/// any lock held, so it may block. A build that is not a test has none of this.
/// A hook at a lane's round start, with the lane's volumes.
#[cfg(test)]
type RoundHook = Arc<dyn Fn(&VolumeSet) + Send + Sync>;

/// A hook at a removal's hide, before its park, with the marker's folder.
#[cfg(test)]
type HideHook = Arc<dyn Fn(&pg_model::rules::Path) + Send + Sync>;

/// A hook once a removal's lanes are parked (its holds taken), before its files are hidden, with the marker's folder. An
/// error fails the hide there, with the park held, as a hide that fails for real does.
#[cfg(test)]
type ParkedHook = Arc<dyn Fn(&pg_model::rules::Path) -> std::result::Result<(), String> + Send + Sync>;

#[cfg(test)]
#[derive(Default)]
pub(crate) struct TestHooks {
    round: Mutex<Option<RoundHook>>,
    hide: Mutex<Option<HideHook>>,
    parked: Mutex<Option<ParkedHook>>,
}

#[cfg(test)]
impl TestHooks {
    /// Runs `hook` at the start of each lane round, with the lane's volumes.
    pub(crate) fn on_round(&self, hook: impl Fn(&VolumeSet) + Send + Sync + 'static) {
        *self.round.lock().unwrap_or_else(PoisonError::into_inner) = Some(Arc::new(hook));
    }

    /// Runs `hook` in the removal job before each marker's park, with the marker's folder.
    pub(crate) fn on_hide(&self, hook: impl Fn(&pg_model::rules::Path) + Send + Sync + 'static) {
        *self.hide.lock().unwrap_or_else(PoisonError::into_inner) = Some(Arc::new(hook));
    }

    /// Runs `hook` once a marker's lanes are parked (its holds taken), before its files are hidden. An `Err` fails that
    /// hide, with the park still held.
    pub(crate) fn on_parked(
        &self,
        hook: impl Fn(&pg_model::rules::Path) -> std::result::Result<(), String> + Send + Sync + 'static,
    ) {
        *self.parked.lock().unwrap_or_else(PoisonError::into_inner) = Some(Arc::new(hook));
    }

    /// The parked hook, if one is set (called by the removal job once its lanes are held).
    pub(crate) fn at_parked(&self, path: &pg_model::rules::Path) -> std::result::Result<(), String> {
        let hook = self.parked.lock().unwrap_or_else(PoisonError::into_inner).clone();
        match hook {
            Some(hook) => hook(path),
            None => Ok(()),
        }
    }

    /// The round hook, if one is set (called by the lane at a round's start).
    pub(crate) fn at_round(&self, volumes: &VolumeSet) {
        let hook = self.round.lock().unwrap_or_else(PoisonError::into_inner).clone();
        if let Some(hook) = hook {
            hook(volumes);
        }
    }

    /// The hide hook, if one is set (called by the removal job before a marker's park).
    pub(crate) fn at_hide(&self, path: &pg_model::rules::Path) {
        let hook = self.hide.lock().unwrap_or_else(PoisonError::into_inner).clone();
        if let Some(hook) = hook {
            hook(path);
        }
    }
}

/// The counts a developer pinned for a run (`RunnerConfig`): a lane's options take them in place of the defaults, and a pinned
/// lane writes no learned profile. Nothing else reads the environment or a flag for them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Pins {
    /// The reads in flight, when pinned.
    pub(crate) readers: Option<NonZeroUsize>,
    /// The decode threads, when pinned.
    pub(crate) workers: Option<NonZeroUsize>,
}

/// What the lanes share: the catalog's path, the thumbnail cache, the cells, the notices and the change bus, the tally of
/// the lanes' notices, the pairing gate, and the job's counts (what the app reads to refresh its views).
pub(crate) struct Shared {
    pub(crate) catalog_path: PathBuf,
    pub(crate) thumbs: Arc<ThumbCache>,
    pub(crate) heights: HeightCell,
    pub(crate) features: FeatureCell,
    pub(crate) notices: Arc<dyn NoticeSink>,
    pub(crate) changes: Arc<Bus<Change>>,
    pub(crate) tally: Tally,
    pub(crate) pairing: PairGate,
    /// The volumes the removals have parked: the lanes on them wait at the start of their rounds.
    pub(crate) parks: Parks,
    /// The passes a round runs (`Runner::set_passes`): read at the start of each round.
    pub(crate) passes: PassCell,
    /// The counts a developer pinned for the run (`RunnerConfig`), in place of the defaults.
    pub(crate) pins: Pins,
    /// The test hooks (none in a build that is not a test).
    #[cfg(test)]
    pub(crate) hooks: Arc<TestHooks>,
    scans_done: AtomicU64,
    imports_done: AtomicU64,
    pub(crate) rounds: AtomicU64,
}

impl Shared {
    pub(crate) fn new(
        catalog_path: PathBuf,
        thumbs: Arc<ThumbCache>,
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
            parks: Parks::default(),
            passes: PassCell::new(PassMask::ALL),
            pins: Pins::default(),
            #[cfg(test)]
            hooks: Arc::default(),
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

#[cfg(test)]
mod parks_tests {
    use std::collections::BTreeSet;

    use super::*;

    fn volumes(of: &[u64]) -> Volumes {
        Volumes { volumes: of.iter().copied().collect(), all: false }
    }

    /// A parked volume blocks only the lanes that read it (D2): a lane on another volume runs, one on it does not, and a
    /// lane that reads every volume does not run either. Quit asked (`|| true`) ends a blocked wait with `false`.
    #[test]
    fn a_parked_volume_blocks_only_the_lanes_that_read_it() {
        let parks = Parks::default();
        parks.park("folder", volumes(&[7]));
        assert_eq!(parks.parked("folder"), Some(volumes(&[7])));
        assert!(parks.wait_clear(&VolumeSet::Only(vec![3]), &|| true), "a lane on another volume runs");
        assert!(!parks.wait_clear(&VolumeSet::Only(vec![7]), &|| true), "a lane on the parked volume waits");
        assert!(!parks.wait_clear(&VolumeSet::All, &|| true), "a lane that reads every volume waits");
    }

    /// Releasing the key lets a lane that waits for it run.
    #[test]
    fn releasing_the_key_lets_the_waiting_lane_run() {
        let parks = Arc::new(Parks::default());
        parks.park("folder", volumes(&[7]));
        let waiting = {
            let parks = parks.clone();
            std::thread::spawn(move || parks.wait_clear(&VolumeSet::Only(vec![7]), &|| false))
        };
        parks.release("folder");
        assert!(waiting.join().expect("the lane's wait ends"));
        assert_eq!(parks.parked("folder"), None);
    }

    /// An all-park blocks every lane, and parking again under the same key replaces the volumes.
    #[test]
    fn an_all_park_blocks_every_lane_until_the_key_is_parked_again_without_it() {
        let parks = Parks::default();
        parks.park("startup removals", Volumes { volumes: BTreeSet::new(), all: true });
        assert!(!parks.wait_clear(&VolumeSet::Only(vec![1]), &|| true), "every lane waits");
        parks.park("startup removals", volumes(&[]));
        assert!(parks.wait_clear(&VolumeSet::Only(vec![1]), &|| true), "replaced: no volume is parked");
    }
}
