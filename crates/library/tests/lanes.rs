//! The job's lanes (`coordinator`): one lane per physical drive its library folders are on, and the cases that collapse
//! them to one. The folders are put on fake drives (`Drives`), so the lanes' shape is checked on any machine.
//!
//! The fake drives all sit on this machine's one real volume, so these tests count the lanes and do not wait for the
//! files to be processed: two lanes that read one volume at once would race each other, which is a test of the fake, not
//! of the lanes. The processing of separate volumes is what the real drives show (`docs/reference/IO.md`).

use std::collections::BTreeSet;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use pg_catalog::Catalog;
use pg_jobs::events::Bus;
use pg_jobs_testkit::{Quiet, RecordingSink};
use pg_library::runner::{Runner, RunnerConfig};
use pg_library::shared::{FeatureCell, HeightCell};
use pg_model::features::Features;
use pg_model::progress::Phase;
use pg_model::removal::Begun;
use pg_model::rules::Path as RulePath;
use pg_model::size::ThumbHeight;
use pg_platform::drive::{Drive, DriveKey, Drives, Kind};
use pg_platform::native::PATH_RULES;
use pg_thumbcache::ThumbCache;

/// The tests' result: any error of the crates they use.
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// How long a test waits for the job to show what it should, and for its threads to end.
const WAIT: Duration = Duration::from_secs(30);

// Each library folder's drive, by the folder's name (its path's spelling differs between the library and the probe). A
/// folder with no entry is a drive that can't be told (an error that is not an unplugged drive), so its root is unknown.
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

/// A run folder with two library folders `a` and `b` (empty: a root's place is all these tests need), its catalog with both
/// added as roots. The folders of the drives are the run folder's children.
fn two_folders(prefix: &str) -> Result<(PathBuf, PathBuf, RulePath, RulePath)> {
    let dir = pg_testkit::run_folder("scratch_library", prefix)?;
    std::fs::create_dir_all(dir.join("a"))?;
    std::fs::create_dir_all(dir.join("b"))?;
    let catalog_path = dir.join("catalog.sqlite");
    let c = Catalog::prepare(&catalog_path, pg_library::versions(), PATH_RULES)?;
    let a = pg_platform::names::normalize_path(&dir.join("a"))?;
    let b = pg_platform::names::normalize_path(&dir.join("b"))?;
    assert!(c.add_root(&a)?);
    assert!(c.add_root(&b)?);
    Ok((dir.clone(), catalog_path, a, b))
}

/// A runner over `catalog_path`, whose drives are asked of `drives`. The thumbnail cache's events are kept by the caller,
/// so the cache's sender lives as long as the runner does.
fn start(
    dir: &Path,
    catalog_path: &Path,
    drives: Fake,
    features: Features,
) -> Result<(Runner, Quiet<pg_thumbcache::Event>)> {
    let id = Catalog::open(catalog_path, PATH_RULES)?.catalog_id()?;
    let (events, quiet) = Quiet::bus();
    let cache = ThumbCache::open(&dir.join("thumbs.sqlite"), &id, None, pg_model::versions::THUMBS_VERSION, events)?;
    let heights = HeightCell::default();
    heights.set(ThumbHeight::new(64).ok_or("64 is not a thumbnail height")?);
    let runner = Runner::start_with_drives(
        catalog_path.to_path_buf(),
        Arc::new(cache),
        heights,
        FeatureCell::new(features),
        Arc::new(Bus::new(64)),
        RecordingSink::new(),
        Arc::new(drives),
        RunnerConfig::default(),
    )?;
    Ok((runner, quiet))
}

/// The folders on two drives make two lanes, one for each drive.
#[test]
fn two_drives_make_two_lanes() -> Result<()> {
    let (dir, catalog_path, a, b) = two_folders("two-drives")?;
    let drives = Fake(vec![("a", disk(1)), ("b", disk(2))]);
    let (runner, _quiet) = start(&dir, &catalog_path, drives, Features::ALL)?;
    runner.scan(vec![a, b]);
    pg_testkit::wait_until(WAIT, || "two lanes", || runner.lanes() == 2);
    assert!(runner.stop(WAIT));
    Ok(())
}

/// Two folders on one drive share one lane: they are one spindle's work, in order.
#[test]
fn folders_on_one_drive_share_a_lane() -> Result<()> {
    let (dir, catalog_path, a, b) = two_folders("one-drive")?;
    let drives = Fake(vec![("a", disk(1)), ("b", disk(1))]);
    let (runner, _quiet) = start(&dir, &catalog_path, drives, Features::ALL)?;
    runner.scan(vec![a, b]);
    pg_testkit::wait_until(WAIT, || "one lane", || runner.lanes() == 1);
    // A lane that is made stays: it is not a second one for a later request.
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(runner.lanes(), 1);
    assert!(runner.stop(WAIT));
    Ok(())
}

/// A folder whose drive can't be told collapses every folder into one lane: fail closed, as one drive did before lanes.
#[test]
fn a_drive_that_cannot_be_told_collapses_the_lanes() -> Result<()> {
    let (dir, catalog_path, a, b) = two_folders("unknown")?;
    // Only `a` is known: `b`'s drive is an error.
    let drives = Fake(vec![("a", disk(1))]);
    let (runner, _quiet) = start(&dir, &catalog_path, drives, Features::ALL)?;
    runner.scan(vec![a, b]);
    pg_testkit::wait_until(WAIT, || "one lane for both", || runner.lanes() == 1);
    assert!(runner.stop(WAIT));
    Ok(())
}

/// The escape hatch: with serial drives on, folders on two drives still make one lane.
#[test]
fn serial_drives_make_one_lane_even_on_two_drives() -> Result<()> {
    let (dir, catalog_path, a, b) = two_folders("serial")?;
    let drives = Fake(vec![("a", disk(1)), ("b", disk(2))]);
    let serial = Features { serial_drives: true, ..Features::ALL };
    let (runner, _quiet) = start(&dir, &catalog_path, drives, serial)?;
    runner.scan(vec![a, b]);
    pg_testkit::wait_until(WAIT, || "one lane", || runner.lanes() == 1);
    assert!(runner.stop(WAIT));
    Ok(())
}

/// A drive with no root left retires its lane: removing its folder from the library leaves the other lane.
#[test]
fn removing_a_drives_last_folder_retires_its_lane() -> Result<()> {
    let (dir, catalog_path, a, b) = two_folders("retire")?;
    let drives = Fake(vec![("a", disk(1)), ("b", disk(2))]);
    let (runner, _quiet) = start(&dir, &catalog_path, drives, Features::ALL)?;
    runner.scan(vec![a.clone(), b.clone()]);
    pg_testkit::wait_until(WAIT, || "two lanes", || runner.lanes() == 2);
    let c = Catalog::open(&catalog_path, PATH_RULES).unwrap();
    let id = c.roots().unwrap().into_iter().find(|root| root.path == b).unwrap().id;
    c.remove_root(id).unwrap();
    runner.wake();
    pg_testkit::wait_until(WAIT, || "the lane of the removed folder to retire", || runner.lanes() == 1);
    assert!(runner.stop(WAIT));
    Ok(())
}

/// A stop ends every lane and the coordinator inside the wait, with no round left running.
#[test]
fn a_stop_ends_every_lane_within_the_wait() -> Result<()> {
    let (dir, catalog_path, a, b) = two_folders("stop")?;
    let drives = Fake(vec![("a", disk(1)), ("b", disk(2))]);
    let (runner, _quiet) = start(&dir, &catalog_path, drives, Features::ALL)?;
    runner.scan(vec![a, b]);
    pg_testkit::wait_until(WAIT, || "two lanes", || runner.lanes() == 2);
    let began = Instant::now();
    assert!(runner.stop(WAIT), "every thread ends");
    assert!(began.elapsed() < WAIT);
    assert!(runner.idle(), "no lane is in a round after the stop");
    Ok(())
}

/// Each lane reports its drive's label with its progress: two drives, two labelled lanes, in the order of their roots.
#[test]
fn each_lane_reports_its_drive_label() -> Result<()> {
    let (dir, catalog_path, a, b) = two_folders("labels")?;
    let drives = Fake(vec![("a", disk(1)), ("b", disk(2))]);
    let (runner, _quiet) = start(&dir, &catalog_path, drives, Features::ALL)?;
    runner.scan(vec![a, b]);
    pg_testkit::wait_until(WAIT, || "two lanes", || runner.lanes() == 2);
    pg_testkit::wait_until(
        WAIT,
        || "both lanes to report their labels",
        || {
            let lanes = runner.lane_progress();
            lanes.len() == 2 && lanes.iter().all(|lane| !lane.label.is_empty())
        },
    );
    let lanes = runner.lane_progress();
    assert_ne!(lanes[0].label, lanes[1].label, "one drive's label is not another's");
    assert!(runner.stop(WAIT));
    Ok(())
}

/// A lane's time since work agrees with its phase in the same read: a lane whose phase runs reports no time since work
/// (`Some(ZERO)`), and once a lane has had work its time since work is known. Waits for that, not for a phase to be seen:
/// a short phase can end between two reads.
#[test]
fn a_lane_reports_its_time_since_work_with_its_phase() -> Result<()> {
    let (dir, catalog_path, a, b) = two_folders("since-work")?;
    let drives = Fake(vec![("a", disk(1)), ("b", disk(2))]);
    let (runner, _quiet) = start(&dir, &catalog_path, drives, Features::ALL)?;
    runner.scan(vec![a, b]);
    pg_testkit::wait_until(WAIT, || "two lanes", || runner.lanes() == 2);
    pg_testkit::wait_until(
        WAIT,
        || "a lane to have had work",
        || runner.lane_progress().iter().any(|lane| lane.since_work.is_some()),
    );
    for lane in runner.lane_progress() {
        if lane.progress.phase != Phase::Idle {
            assert_eq!(lane.since_work, Some(Duration::ZERO), "a running phase has no time since work");
        }
    }
    assert!(runner.stop(WAIT));
    Ok(())
}

/// A pause of one lane (L7) leaves the other lanes running: only the paused lane reports itself paused, the run's global
/// pause is not set, and a number that no lane has is refused.
#[test]
fn a_per_lane_pause_pauses_only_its_lane() -> Result<()> {
    let (dir, catalog_path, a, b) = two_folders("lane-pause")?;
    let drives = Fake(vec![("a", disk(1)), ("b", disk(2))]);
    let (runner, _quiet) = start(&dir, &catalog_path, drives, Features::ALL)?;
    runner.scan(vec![a, b]);
    pg_testkit::wait_until(WAIT, || "two lanes", || runner.lanes() == 2);
    let first = runner.lane_progress()[0].number;
    let second = runner.lane_progress()[1].number;
    assert!(runner.set_lane_paused(first, true));
    pg_testkit::wait_until(
        WAIT,
        || "the first lane to report itself paused",
        || {
            let lanes = runner.lane_progress();
            lanes.iter().any(|lane| lane.number == first && lane.paused)
        },
    );
    let lanes = runner.lane_progress();
    assert!(lanes.iter().any(|lane| lane.number == second && !lane.paused), "the other lane runs on");
    assert!(!runner.is_paused(), "a lane's pause is not the run's pause");
    assert!(!runner.set_lane_paused(first.wrapping_add(1_000), true), "no such lane");
    assert!(runner.set_lane_paused(first, false));
    assert!(runner.stop(WAIT));
    Ok(())
}

/// A removal parks its folder's lanes as the app begins it (`park_removal`), and the removal job releases them once the
/// folder is hidden: a scan after the removal runs a round, so no park was left behind (section 1, D1 and D2). The folder
/// is empty, so no file of it has a volume and every lane is parked (D2's case).
#[test]
fn a_removal_releases_its_parks_once_its_folder_is_hidden() -> Result<()> {
    let (dir, catalog_path, a, b) = two_folders("removal_release")?;
    let drives = Fake(vec![("a", disk(1)), ("b", disk(2))]);
    let (runner, _quiet) = start(&dir, &catalog_path, drives, Features::ALL)?;
    runner.scan(vec![a.clone(), b.clone()]);
    pg_testkit::wait_until(WAIT, || "two lanes", || runner.lanes() == 2);
    pg_testkit::wait_until(WAIT, || "a first round", || runner.rounds() > 0);
    let catalog = Catalog::open(&catalog_path, PATH_RULES)?;
    let root = catalog.root_id(&a)?.ok_or("the folder is a root")?;
    let Begun::Started { path, volumes } = catalog.begin_removal(root, &a)? else {
        return Err("the removal did not start".into());
    };
    assert!(volumes.all, "an empty folder has no volume: every lane is parked");
    runner.park_removal(&path, &volumes);
    runner.wake_removals();
    pg_testkit::wait_until(WAIT, || "the removal finished", || catalog.removal_markers().is_ok_and(|m| m.is_empty()));
    let before = runner.rounds();
    runner.scan(vec![b]);
    pg_testkit::wait_until(WAIT, || "a round after the removal", || runner.rounds() > before);
    assert!(runner.stop(WAIT));
    Ok(())
}
