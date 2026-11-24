//! The removals' parks, checked on the job itself (section 1), through its hooks (`shared::TestHooks`): a lane's round is
//! held at its start, and a removal is held before it hides its folder. The folders sit on fake drives, all on this
//! machine's one real volume, so a park of a folder's volume parks every lane here. What a park does to a lane on another
//! volume is checked by the registry's own tests (`shared::parks_tests`).
//!
//! A test that checks that something does NOT happen waits `SETTLE` first: a lane that could start its round has had that
//! long to do it. Such a check is bounded, not a proof, and the step after it shows the thing happens once allowed.

use std::collections::BTreeSet;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::Duration;

use pg_catalog::Catalog;
use pg_model::features::Features;
use pg_model::removal::{Begun, Volumes};
use pg_model::rules::Path as RulePath;
use pg_model::size::ThumbHeight;
use pg_platform::drive::{Drive, DriveKey, Drives, Kind};
use pg_platform::native::PATH_RULES;
use pg_thumbcache::ThumbCache;

use crate::runner::Runner;
use crate::shared::{FeatureCell, HeightCell, TestHooks};

/// The tests' result: any error of the crates they use.
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// How long a test waits for the job to show what it should.
const WAIT: Duration = Duration::from_secs(30);
/// How long a check that something does not happen waits before it looks.
const SETTLE: Duration = Duration::from_millis(300);

/// Each library folder's drive, by the folder's name.
struct Fake(Vec<(&'static str, DriveKey)>);

impl Drives for Fake {
    fn of(&self, path: &Path) -> io::Result<Drive> {
        let name = path.file_name().and_then(|name| name.to_str()).unwrap_or_default();
        let (_, key) = self
            .0
            .iter()
            .find(|(folder, _)| *folder == name)
            .ok_or_else(|| io::Error::other("no drive is known for this path"))?;
        Ok(Drive { key: key.clone(), kind: Kind::Rotational, label: format!("{key:?}") })
    }
}

/// The drive of one physical disk, by its number.
fn disk(number: u64) -> DriveKey {
    DriveKey::Disks(BTreeSet::from([number]))
}

/// A run folder with two empty library folders `a` and `b`, their catalog with both added as roots.
fn two_folders(prefix: &str) -> Result<(PathBuf, PathBuf, RulePath, RulePath)> {
    let dir = pg_testkit::run_folder("scratch_library", prefix)?;
    std::fs::create_dir_all(dir.join("a"))?;
    std::fs::create_dir_all(dir.join("b"))?;
    let catalog_path = dir.join("catalog.sqlite");
    let c = Catalog::prepare(&catalog_path, crate::versions(), PATH_RULES)?;
    let a = pg_platform::names::normalize_path(&dir.join("a"))?;
    let b = pg_platform::names::normalize_path(&dir.join("b"))?;
    assert!(c.add_root(&a)?);
    assert!(c.add_root(&b)?);
    Ok((dir.clone(), catalog_path, a, b))
}

/// A runner over `catalog_path` with `hooks` in place, whose drives are asked of `drives`.
fn start(
    dir: &Path,
    catalog_path: &Path,
    drives: Fake,
    hooks: Arc<TestHooks>,
) -> Result<(Runner, pg_jobs_testkit::Quiet<pg_thumbcache::Event>)> {
    let id = Catalog::open(catalog_path, PATH_RULES)?.catalog_id()?;
    let (events, quiet) = pg_jobs_testkit::Quiet::bus();
    let cache = ThumbCache::open(&dir.join("thumbs.sqlite"), &id, None, pg_model::versions::THUMBS_VERSION, events)?;
    let heights = HeightCell::default();
    heights.set(ThumbHeight::new(64).ok_or("64 is not a thumbnail height")?);
    let runner = Runner::start_hooked(
        catalog_path.to_path_buf(),
        Arc::new(cache),
        heights,
        FeatureCell::new(Features::ALL),
        Arc::new(drives),
        hooks,
    )?;
    Ok((runner, quiet))
}

/// Opens `gate` when it is dropped. A test declares it after its runner, so it drops first: a failed assertion then does not
/// leave a lane held in the gate, which the runner's shutdown would wait for.
struct Release(Arc<Gate>);

impl Drop for Release {
    fn drop(&mut self) {
        self.0.open();
    }
}

/// Counts the lanes' rounds as they start. Set before the runner starts, so no round is missed.
fn counted(hooks: &TestHooks) -> Arc<AtomicUsize> {
    let rounds = Arc::new(AtomicUsize::new(0));
    let counter = rounds.clone();
    hooks.on_round(move |_| {
        counter.fetch_add(1, Ordering::SeqCst);
    });
    rounds
}

/// A one-shot gate: the hook that `enter`s it waits until the test `open`s it. `entered` says the hook was reached.
#[derive(Default)]
struct Gate {
    state: Mutex<GateState>,
    changed: Condvar,
}

#[derive(Default)]
struct GateState {
    entered: bool,
    open: bool,
}

impl Gate {
    fn enter(&self) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.entered = true;
        self.changed.notify_all();
        while !state.open {
            state = self.changed.wait(state).unwrap_or_else(PoisonError::into_inner);
        }
    }

    fn entered(&self) -> bool {
        self.state.lock().unwrap_or_else(PoisonError::into_inner).entered
    }

    fn open(&self) {
        self.state.lock().unwrap_or_else(PoisonError::into_inner).open = true;
        self.changed.notify_all();
    }
}

/// Begins the removal of `folder` as the app does (`Catalog::begin_removal`), and returns its parks.
fn begin(catalog_path: &Path, folder: &RulePath) -> Result<(RulePath, Volumes)> {
    let catalog = Catalog::open(catalog_path, PATH_RULES)?;
    let root = catalog.root_id(folder)?.ok_or("the folder is a root")?;
    match catalog.begin_removal(root, folder)? {
        Begun::Started { path, volumes } => Ok((path, volumes)),
        other => Err(format!("the removal did not start: {other:?}").into()),
    }
}

/// How many removal markers the catalog holds.
fn markers(catalog_path: &Path) -> Result<usize> {
    Ok(Catalog::open(catalog_path, PATH_RULES)?.removal_markers()?.len())
}

/// The removal of a folder waits for the round its lane is in: the job is held at its hide while the lane's round is held
/// (the round's hook, as a pass would hold it), and the marker stays until the round ends (D1).
#[test]
fn a_removal_waits_for_the_round_its_lanes_are_in() -> Result<()> {
    let (dir, catalog_path, a, _b) = two_folders("wait_for_round")?;
    let hooks = Arc::new(TestHooks::default());
    let held = Arc::new(Gate::default());
    let first = Arc::new(AtomicBool::new(true));
    {
        let (held, first) = (held.clone(), first.clone());
        hooks.on_round(move |_| {
            if first.swap(false, Ordering::SeqCst) {
                held.enter();
            }
        });
    }
    let (runner, _quiet) = start(&dir, &catalog_path, Fake(vec![("a", disk(1))]), hooks)?;
    let _release = Release(held.clone());
    runner.scan(vec![a.clone()]);
    pg_testkit::wait_until(WAIT, || "the lane is in its round", || held.entered());
    let (path, volumes) = begin(&catalog_path, &a)?;
    runner.park_removal(&path, &volumes);
    runner.wake_removals();
    std::thread::sleep(SETTLE);
    assert_eq!(markers(&catalog_path)?, 1, "the removal waits while the lane's round runs");
    held.open();
    pg_testkit::wait_until(WAIT, || "the removal finished", || markers(&catalog_path).is_ok_and(|n| n == 0));
    assert!(runner.stop(WAIT));
    Ok(())
}

/// A removal an earlier run left holds its lanes from the start (D5): a lane on the folder's volumes starts no round until
/// the folder is hidden. The removal job is held at its hide, so the hold is seen while the folder is still there.
#[test]
fn a_removal_an_earlier_run_left_holds_the_lanes_until_its_folder_is_hidden() -> Result<()> {
    let (dir, catalog_path, a, b) = two_folders("earlier_removal")?;
    // The earlier run began the removal and stopped before it hid the folder: the marker is kept.
    begin(&catalog_path, &a)?;
    let hooks = Arc::new(TestHooks::default());
    let held = Arc::new(Gate::default());
    {
        let held = held.clone();
        hooks.on_hide(move |_| held.enter());
    }
    let rounds = counted(&hooks);
    let (runner, _quiet) = start(&dir, &catalog_path, Fake(vec![("a", disk(1)), ("b", disk(2))]), hooks)?;
    let _release = Release(held.clone());
    pg_testkit::wait_until(WAIT, || "the removal is held", || held.entered());
    runner.scan(vec![b]);
    std::thread::sleep(SETTLE);
    assert_eq!(rounds.load(Ordering::SeqCst), 0, "no lane starts a round before the folder is hidden");
    held.open();
    pg_testkit::wait_until(WAIT, || "a round once the folder is hidden", || rounds.load(Ordering::SeqCst) > 0);
    pg_testkit::wait_until(WAIT, || "the removal finished", || markers(&catalog_path).is_ok_and(|n| n == 0));
    assert!(runner.stop(WAIT));
    Ok(())
}

/// A lane spawned while a removal is parked starts no round until the removal releases its parks (D6): the lane for
/// `b`'s drive is new when the removal is held before it hides `a`.
#[test]
fn a_lane_spawned_during_a_removal_waits_for_it() -> Result<()> {
    let (dir, catalog_path, a, b) = two_folders("spawned_during")?;
    let hooks = Arc::new(TestHooks::default());
    let held = Arc::new(Gate::default());
    {
        let held = held.clone();
        hooks.on_hide(move |_| held.enter());
    }
    let rounds = counted(&hooks);
    let (runner, _quiet) = start(&dir, &catalog_path, Fake(vec![("a", disk(1)), ("b", disk(2))]), hooks)?;
    let _release = Release(held.clone());
    let (path, volumes) = begin(&catalog_path, &a)?;
    runner.park_removal(&path, &volumes);
    runner.wake_removals();
    pg_testkit::wait_until(WAIT, || "the removal is held", || held.entered());
    // The lanes the library starts run their own first rounds, before any of this: the count is taken after them.
    std::thread::sleep(SETTLE);
    let before = rounds.load(Ordering::SeqCst);
    runner.scan(vec![b]);
    std::thread::sleep(SETTLE);
    assert_eq!(rounds.load(Ordering::SeqCst), before, "a lane spawned during the removal starts no round before it");
    held.open();
    pg_testkit::wait_until(WAIT, || "a round after the hide", || rounds.load(Ordering::SeqCst) > before);
    pg_testkit::wait_until(WAIT, || "the removal finished", || markers(&catalog_path).is_ok_and(|n| n == 0));
    assert!(runner.stop(WAIT));
    Ok(())
}
