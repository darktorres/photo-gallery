//! The removals' parks, checked on the job itself (section 1), through its hooks (`shared::TestHooks`): a lane's round is
//! held at its start, and a removal is held before it hides its folder. The folders sit on fake drives, all on this
//! machine's one real volume, so a park of a folder's volume parks every lane here. What a park does to a lane on another
//! volume is checked by the registry's own tests (`shared::parks_tests`), and, with a second real volume, by
//! `a_park_on_one_volume_leaves_a_lane_on_another_running`.
//!
//! Two facts shape these tests. A lane retires when its drive's last folder is removed, and a lone lane is given every
//! volume; so a test that removes a folder keeps a second folder on the same drive. And a wake with no work starts no round,
//! so a test that needs a round asks for one with a scan.
//!
//! A test that checks that something does NOT happen waits `SETTLE` first: a lane that could start its round has had that
//! long to do it. Such a check is bounded, not a proof, and the step after it shows the thing happens once allowed.

use std::collections::BTreeSet;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant};

use pg_catalog::Catalog;
use pg_jobs::control::Control;
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
        Ok(Drive { key: key.clone(), kind: Kind::Rotational, label: format!("{key:?}"), identity: None })
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

/// Creates the library folder `name` under `dir` and adds it to the catalog as a root.
fn add_folder(dir: &Path, catalog_path: &Path, name: &str) -> Result<RulePath> {
    let folder = dir.join(name);
    std::fs::create_dir_all(&folder)?;
    let path = pg_platform::names::normalize_path(&folder)?;
    assert!(Catalog::open(catalog_path, PATH_RULES)?.add_root(&path)?);
    Ok(path)
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

/// Counts the lanes' rounds as they start. Set before the runner starts, so no round is missed.
fn counted(hooks: &TestHooks) -> Arc<AtomicUsize> {
    let rounds = Arc::new(AtomicUsize::new(0));
    let counter = rounds.clone();
    hooks.on_round(move |_| {
        counter.fetch_add(1, Ordering::SeqCst);
    });
    rounds
}

/// The removal's hide over 100,000 files, timed with no lane to wait for: the lower bound of the park window, which the cost
/// prediction in section 1 extrapolates from. A measurement, not a check. The files go on `PG_TEST_OTHER_VOLUME` when it is
/// set (the volume the section's earlier measurement used), else in the run folder.
/// `cargo test -p pg-library --lib -- --ignored hide_window --nocapture`.
#[test]
#[ignore = "a measurement: 100,000 files, run with --ignored --nocapture"]
fn hide_window_for_100k_files() -> Result<()> {
    let base = match std::env::var_os(pg_testkit::inputs::OTHER_VOLUME) {
        Some(other) => PathBuf::from(other),
        None => pg_testkit::run_folder("scratch_library", "hide-100k")?,
    };
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos();
    let dir = base.join(format!("pg-hide-100k-{}-{stamp}", std::process::id()));
    let folder = dir.join("a");
    std::fs::create_dir_all(&folder)?;
    for i in 0..100_000 {
        std::fs::write(folder.join(format!("{i:06}.png")), b"x")?;
    }
    let catalog_path = dir.join("catalog.sqlite");
    let c = Catalog::prepare(&catalog_path, crate::versions(), PATH_RULES)?;
    let a = pg_platform::names::normalize_path(&folder)?;
    assert!(c.add_root(&a)?);
    drop(c);
    let scanned = Instant::now();
    scan_into(&catalog_path, &a)?;
    println!("scan of 100,000 files: {:?}", scanned.elapsed());
    let c = Catalog::open(&catalog_path, PATH_RULES)?;
    let hidden_in = Instant::now();
    let hidden = c.hide_pages(&a)?;
    println!("hide of {hidden} files: {:?}", hidden_in.elapsed());
    drop(c);
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

/// The held round of a lane's start: `hooks` holds the first round at its start, until `held` is opened.
fn hold_first_round(hooks: &TestHooks, held: &Arc<Gate>) {
    let first = Arc::new(AtomicBool::new(true));
    let held = held.clone();
    hooks.on_round(move |_| {
        if first.swap(false, Ordering::SeqCst) {
            held.enter();
        }
    });
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

/// Opens `gate` when it is dropped. A test declares it after its runner, so it drops first: a failed assertion then does not
/// leave a lane held in the gate, which the runner's shutdown would wait for.
struct Release(Arc<Gate>);

impl Drop for Release {
    fn drop(&mut self) {
        self.0.open();
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

/// A small real picture, as PNG bytes: the import decodes what the scan found, so the files have to be pictures.
fn picture() -> Result<Vec<u8>> {
    let pixels: Vec<u8> =
        (0..8 * 8 * 3).map(|i| u8::try_from(i % 251)).collect::<std::result::Result<_, std::num::TryFromIntError>>()?;
    Ok(pg_codec::fixtures::png_rgb(8, 8, &pixels)?)
}

/// A run folder with the library folder `a` holding `pictures` pictures, and its catalog with `a` as a root. The pictures
/// are catalogued by a scan only when `scanned`; otherwise a scan of the job does it.
fn folder_with_pictures(prefix: &str, pictures: usize, scanned: bool) -> Result<(PathBuf, PathBuf, RulePath)> {
    let dir = pg_testkit::run_folder("scratch_library", prefix)?;
    let folder = dir.join("a");
    std::fs::create_dir_all(&folder)?;
    let png = picture()?;
    for i in 0..pictures {
        std::fs::write(folder.join(format!("{i:05}.png")), &png)?;
    }
    let catalog_path = dir.join("catalog.sqlite");
    let c = Catalog::prepare(&catalog_path, crate::versions(), PATH_RULES)?;
    let a = pg_platform::names::normalize_path(&folder)?;
    assert!(c.add_root(&a)?);
    drop(c);
    if scanned {
        scan_into(&catalog_path, &a)?;
    }
    Ok((dir, catalog_path, a))
}

/// Scans `folder` into the catalog: its files are catalogued, and not imported.
fn scan_into(catalog_path: &Path, folder: &RulePath) -> Result<()> {
    let mut c = Catalog::open(catalog_path, PATH_RULES)?;
    pg_scan::scan(&mut c, folder, &Control::new())?;
    Ok(())
}

/// The removal of a folder waits for the round its lane is in: the job is held at its hide while the lane's round is held
/// (the round's hook, as a pass would hold it), and the marker stays until the round ends (D1).
#[test]
fn a_removal_waits_for_the_round_its_lanes_are_in() -> Result<()> {
    let (dir, catalog_path, a, _b) = two_folders("wait_for_round")?;
    // A second folder on `a`'s drive keeps the lane alive through the removal.
    add_folder(&dir, &catalog_path, "c")?;
    let hooks = Arc::new(TestHooks::default());
    let held = Arc::new(Gate::default());
    hold_first_round(&hooks, &held);
    let (runner, _quiet) = start(&dir, &catalog_path, Fake(vec![("a", disk(1)), ("c", disk(1))]), hooks)?;
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

/// A stop that reaches a removal while it waits for its lanes' rounds leaves the marker and the park in place: no round runs
/// on the folder meanwhile, and the next try finishes the removal (section 1, the park that stops).
#[test]
fn a_stop_while_the_removal_waits_keeps_the_folder_parked() -> Result<()> {
    let (dir, catalog_path, a, _b) = two_folders("stop_keeps_park")?;
    let c = add_folder(&dir, &catalog_path, "c")?;
    let hooks = Arc::new(TestHooks::default());
    let held = Arc::new(Gate::default());
    hold_first_round(&hooks, &held);
    let (runner, _quiet) = start(&dir, &catalog_path, Fake(vec![("a", disk(1)), ("c", disk(1))]), hooks)?;
    let _release = Release(held.clone());
    runner.scan(vec![a.clone()]);
    pg_testkit::wait_until(WAIT, || "the lane is in its round", || held.entered());
    let (path, volumes) = begin(&catalog_path, &a)?;
    runner.park_removal(&path, &volumes);
    runner.wake_removals();
    std::thread::sleep(SETTLE);
    // The stop reaches the removal while it waits for the round: its wait ends, and it leaves the marker parked.
    runner.lanes.removal_ctl.request_stop();
    std::thread::sleep(SETTLE);
    held.open();
    pg_testkit::wait_until(WAIT, || "the round ended", || runner.rounds() >= 1);
    std::thread::sleep(SETTLE);
    assert_eq!(markers(&catalog_path)?, 1, "a stopped removal leaves its marker");
    let before = runner.rounds();
    runner.scan(vec![c.clone()]);
    std::thread::sleep(SETTLE);
    assert_eq!(runner.rounds(), before, "the park stands after the stop: the lane runs no round");
    // The next try: the stop is cleared, and the removal runs to its end.
    runner.lanes.removal_ctl.clear_stop();
    runner.wake_removals();
    pg_testkit::wait_until(WAIT, || "the removal finished", || markers(&catalog_path).is_ok_and(|n| n == 0));
    pg_testkit::wait_until(
        WAIT,
        || "a round once the removal finished",
        || {
            runner.scan(vec![c.clone()]);
            runner.rounds() > before
        },
    );
    assert!(runner.stop(WAIT));
    Ok(())
}

/// A hide that fails with its lanes parked keeps the park: no round runs on the folder until the hide completes, and the
/// next try finishes it (section 1, the failure path of the hide).
#[test]
fn a_failed_hide_keeps_the_folder_parked_until_it_completes() -> Result<()> {
    let (dir, catalog_path, a, _b) = two_folders("failed_hide")?;
    let c = add_folder(&dir, &catalog_path, "c")?;
    let hooks = Arc::new(TestHooks::default());
    let fails = Arc::new(AtomicBool::new(true));
    {
        let fails = fails.clone();
        hooks.on_parked(move |_| {
            if fails.swap(false, Ordering::SeqCst) { Err("the disk refused the write".to_owned()) } else { Ok(()) }
        });
    }
    let (runner, _quiet) = start(&dir, &catalog_path, Fake(vec![("a", disk(1)), ("c", disk(1))]), hooks)?;
    runner.scan(vec![a.clone(), c.clone()]);
    pg_testkit::wait_until(WAIT, || "the lane has run", || runner.rounds() >= 1 && runner.idle());
    let (path, volumes) = begin(&catalog_path, &a)?;
    runner.park_removal(&path, &volumes);
    runner.wake_removals();
    pg_testkit::wait_until(WAIT, || "the hide was tried", || !fails.load(Ordering::SeqCst));
    std::thread::sleep(SETTLE);
    assert_eq!(markers(&catalog_path)?, 1, "a failed hide keeps the marker");
    let before = runner.rounds();
    runner.scan(vec![c.clone()]);
    std::thread::sleep(SETTLE);
    assert_eq!(runner.rounds(), before, "a failed hide keeps the folder parked: the lane runs no round");
    runner.wake_removals();
    pg_testkit::wait_until(WAIT, || "the removal finished", || markers(&catalog_path).is_ok_and(|n| n == 0));
    pg_testkit::wait_until(
        WAIT,
        || "a round once the hide completed",
        || {
            runner.scan(vec![c.clone()]);
            runner.rounds() > before
        },
    );
    assert!(runner.stop(WAIT));
    Ok(())
}

/// Two holds on one volume, a crop's (`Lanes::park`) and a removal's (`Lanes::park_owners`), both stand: the lane runs no
/// round until both are released (section 1, the overlapping parks).
#[test]
fn a_crop_park_and_a_removal_park_on_one_volume_both_hold_the_lane() -> Result<()> {
    let (dir, catalog_path, a, _b) = two_folders("overlap_parks")?;
    let (runner, _quiet) = start(&dir, &catalog_path, Fake(vec![("a", disk(1))]), Arc::new(TestHooks::default()))?;
    runner.scan(vec![a.clone()]);
    pg_testkit::wait_until(WAIT, || "the lane has run", || runner.rounds() >= 1 && runner.idle());
    let volumes = BTreeSet::from([pg_scan::root_volume(PATH_RULES, &a)?]);
    let crop = runner.lanes.park(&volumes).ok_or("the crop's park did not hold")?;
    let removal = runner
        .lanes
        .park_owners(&Volumes { volumes: volumes.clone(), all: false }, &Control::new())
        .ok_or("the removal's park did not hold")?;
    let before = runner.rounds();
    drop(crop);
    runner.scan(vec![a.clone()]);
    std::thread::sleep(SETTLE);
    assert_eq!(runner.rounds(), before, "the removal's hold stands once the crop's is released");
    drop(removal);
    pg_testkit::wait_until(
        WAIT,
        || "a round once both are released",
        || {
            runner.scan(vec![a.clone()]);
            runner.rounds() > before
        },
    );
    assert!(runner.stop(WAIT));
    Ok(())
}

/// A removal begun after a lane's first round, with its folder's pictures catalogued and waiting to be imported: a round
/// the lane starts while the removal is held imports none of them (section 1, P1). The pictures are catalogued after the
/// first round, so nothing is waiting to be imported then.
#[test]
fn a_lane_in_a_round_is_not_quiescent_until_the_round_ends() -> Result<()> {
    let (dir, catalog_path, _a) = folder_with_pictures("quiescent_round", 3, false)?;
    let hooks = Arc::new(TestHooks::default());
    let held = Arc::new(Gate::default());
    hold_first_round(&hooks, &held);
    let (runner, _quiet) = start(&dir, &catalog_path, Fake(vec![("a", disk(1))]), hooks)?;
    let _release = Release(held.clone());
    pg_testkit::wait_until(WAIT, || "the lane's first round is held", || held.entered());
    assert!(!runner.quiescent(), "a lane in a round is not quiet");
    held.open();
    pg_testkit::wait_until(WAIT, || "the round ends and the job is quiet", || runner.quiescent());
    assert!(runner.stop(WAIT));
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

#[test]
fn a_round_held_when_its_folder_is_removed_imports_none_of_it() -> Result<()> {
    let (dir, catalog_path, a) = folder_with_pictures("p1_import", 20, false)?;
    // A second folder on `a`'s drive keeps the lane alive through the removal.
    let c = add_folder(&dir, &catalog_path, "c")?;
    let hooks = Arc::new(TestHooks::default());
    let hide = Arc::new(Gate::default());
    {
        let hide = hide.clone();
        hooks.on_hide(move |_| hide.enter());
    }
    let (runner, _quiet) = start(&dir, &catalog_path, Fake(vec![("a", disk(1)), ("c", disk(1))]), hooks)?;
    let _hide_release = Release(hide.clone());
    pg_testkit::wait_until(WAIT, || "the lane has run its first round", || runner.rounds() >= 1 && runner.idle());
    let imports_before = runner.generation().imports;
    scan_into(&catalog_path, &a)?;
    let catalogued = Catalog::open(&catalog_path, PATH_RULES)?.files_under(&a)?;
    assert_eq!(catalogued.len(), 20, "the pictures are catalogued");
    let (path, volumes) = begin(&catalog_path, &a)?;
    runner.park_removal(&path, &volumes);
    runner.wake_removals();
    pg_testkit::wait_until(WAIT, || "the removal is held", || hide.entered());
    // A round of the lane now would import the pictures before they are hidden: the scan asks for one.
    runner.scan(vec![c.clone()]);
    std::thread::sleep(SETTLE);
    assert_eq!(
        runner.generation().imports,
        imports_before,
        "no picture of the removed folder was imported in the window before its hide"
    );
    hide.open();
    pg_testkit::wait_until(WAIT, || "the removal finished", || markers(&catalog_path).is_ok_and(|n| n == 0));
    assert!(runner.stop(WAIT));
    Ok(())
}

/// A removal begun while a scan of its folder is held at the start of its round: the scan puts none of the folder's files
/// back, and at the end the folder is not a root and holds no file (the manual check, "remove a folder during a large
/// scan"). The folder is the only one on its drive, so its lane retires with the removal.
#[test]
fn a_removal_begun_during_a_scan_leaves_no_file_of_the_folder() -> Result<()> {
    let (dir, catalog_path, a) = folder_with_pictures("remove_during_scan", 30, false)?;
    let hooks = Arc::new(TestHooks::default());
    let held = Arc::new(Gate::default());
    hold_first_round(&hooks, &held);
    let (runner, _quiet) = start(&dir, &catalog_path, Fake(vec![("a", disk(1))]), hooks)?;
    let _release = Release(held.clone());
    runner.scan(vec![a.clone()]);
    pg_testkit::wait_until(WAIT, || "the scan's round is held", || held.entered());
    let (path, volumes) = begin(&catalog_path, &a)?;
    runner.park_removal(&path, &volumes);
    runner.wake_removals();
    std::thread::sleep(SETTLE);
    held.open();
    pg_testkit::wait_until(WAIT, || "the removal finished", || markers(&catalog_path).is_ok_and(|n| n == 0));
    let catalog = Catalog::open(&catalog_path, PATH_RULES)?;
    assert!(catalog.files_under(&a)?.is_empty(), "no file of the removed folder is left");
    assert!(catalog.root_id(&a)?.is_none(), "the removed folder is no longer a root");
    assert!(runner.stop(WAIT));
    Ok(())
}

/// A park holds only the lanes on its volume (D2): a removal of a folder on one volume is parked while a lane on another
/// volume keeps running its rounds. This needs a second real volume: `PG_TEST_OTHER_VOLUME` names a directory on it, and
/// without it the test is skipped, which its output says.
#[test]
fn a_park_on_one_volume_leaves_a_lane_on_another_running() -> Result<()> {
    let Some(other) = std::env::var_os(pg_testkit::inputs::OTHER_VOLUME).map(PathBuf::from) else {
        eprintln!("skipped: PG_TEST_OTHER_VOLUME names no directory on a second real volume");
        return Ok(());
    };
    let (dir, catalog_path, a) = folder_with_pictures("other_volume", 5, false)?;
    // The second folder lives on the other volume. Both are catalogued, so each lane's volume is known.
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos();
    let elsewhere = other.join(format!("pg-park-{}-{stamp}", std::process::id()));
    let b_dir = elsewhere.join("b");
    std::fs::create_dir_all(&b_dir)?;
    let png = picture()?;
    for i in 0..5 {
        std::fs::write(b_dir.join(format!("{i:05}.png")), &png)?;
    }
    let b = pg_platform::names::normalize_path(&b_dir)?;
    // A third folder on `a`'s drive: removing `a` must not retire its lane. A lone lane is given every volume, and then a
    // park of `a`'s volume would park the other lane too; the test is about two lanes on two volumes.
    add_folder(&dir, &catalog_path, "c")?;
    assert!(Catalog::open(&catalog_path, PATH_RULES)?.add_root(&b)?);
    scan_into(&catalog_path, &a)?;
    scan_into(&catalog_path, &b)?;
    let volume_a = pg_scan::root_volume(PATH_RULES, &a)?;
    assert_ne!(volume_a, pg_scan::root_volume(PATH_RULES, &b)?, "the second folder is on another volume");

    let seen_on_a = Arc::new(Mutex::new(Vec::<bool>::new()));
    let hooks = Arc::new(TestHooks::default());
    {
        let seen = seen_on_a.clone();
        hooks.on_round(move |volumes| {
            seen.lock().unwrap_or_else(PoisonError::into_inner).push(volumes.contains(volume_a));
        });
    }
    let held = Arc::new(Gate::default());
    {
        let held = held.clone();
        hooks.on_hide(move |_| held.enter());
    }
    let (runner, _quiet) =
        start(&dir, &catalog_path, Fake(vec![("a", disk(1)), ("c", disk(1)), ("b", disk(2))]), hooks)?;
    let _release = Release(held.clone());
    let (path, volumes) = begin(&catalog_path, &a)?;
    runner.park_removal(&path, &volumes);
    runner.wake_removals();
    pg_testkit::wait_until(WAIT, || "the removal is held", || held.entered());
    seen_on_a.lock().unwrap_or_else(PoisonError::into_inner).clear();
    runner.scan(vec![b]);
    pg_testkit::wait_until(
        WAIT,
        || "the lane on the other volume runs a round",
        || !seen_on_a.lock().unwrap_or_else(PoisonError::into_inner).is_empty(),
    );
    let seen = seen_on_a.lock().unwrap_or_else(PoisonError::into_inner).clone();
    assert!(seen.iter().all(|on_a| !on_a), "no round on the parked volume while the removal holds it: {seen:?}");
    held.open();
    pg_testkit::wait_until(WAIT, || "the removal finished", || markers(&catalog_path).is_ok_and(|n| n == 0));
    assert!(runner.stop(WAIT));
    let _ = std::fs::remove_dir_all(&elsewhere);
    Ok(())
}

/// The park's wait while an import runs: the time from the removal's park to the parked hook, which runs once the lanes'
/// rounds have ended. A measurement, not a check: `cargo test -p pg-library --lib -- --ignored park_wait --nocapture`.
#[test]
#[ignore = "a measurement: run with --ignored --nocapture"]
fn park_wait_under_an_import() -> Result<()> {
    let (dir, catalog_path, a) = folder_with_pictures("park_wait", 3000, true)?;
    let hooks = Arc::new(TestHooks::default());
    let parked_at = Arc::new(Mutex::new(None::<Instant>));
    {
        let parked_at = parked_at.clone();
        hooks.on_parked(move |_| {
            parked_at.lock().unwrap_or_else(PoisonError::into_inner).get_or_insert_with(Instant::now);
            Ok(())
        });
    }
    let (runner, _quiet) = start(&dir, &catalog_path, Fake(vec![("a", disk(1))]), hooks)?;
    pg_testkit::wait_until(WAIT, || "the import is running", || !runner.idle());
    let imports_at_begin = runner.generation().imports;
    let (path, volumes) = begin(&catalog_path, &a)?;
    let busy_at_park = !runner.idle();
    let began = Instant::now();
    runner.park_removal(&path, &volumes);
    runner.wake_removals();
    pg_testkit::wait_until(
        WAIT,
        || "the removal is parked",
        || parked_at.lock().unwrap_or_else(PoisonError::into_inner).is_some(),
    );
    let parked = parked_at.lock().unwrap_or_else(PoisonError::into_inner).unwrap_or(began);
    println!(
        "park wait under an import of 3000 pictures (busy when the removal began: {busy_at_park}, imports done: \
         {imports_at_begin}): {:?}",
        parked.saturating_duration_since(began)
    );
    pg_testkit::wait_until(WAIT, || "the removal finished", || markers(&catalog_path).is_ok_and(|n| n == 0));
    assert!(runner.stop(WAIT));
    Ok(())
}
