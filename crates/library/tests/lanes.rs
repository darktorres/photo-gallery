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
use pg_library::runner::{FeatureCell, HeightCell, Runner};
use pg_model::features::Features;
use pg_model::rules::Path as RulePath;
use pg_model::size::ThumbHeight;
use pg_platform::drive::{Drive, DriveKey, Drives, Kind};
use pg_platform::native::PATH_RULES;
use pg_thumbcache::ThumbCache;

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
#[expect(clippy::unwrap_used, reason = "a test's setup: its failure is the test's")]
fn two_folders(prefix: &str) -> (PathBuf, PathBuf, RulePath, RulePath) {
    let dir = pg_testkit::run_folder("scratch_lanes", prefix).unwrap();
    std::fs::create_dir_all(dir.join("a")).unwrap();
    std::fs::create_dir_all(dir.join("b")).unwrap();
    let catalog_path = dir.join("catalog.sqlite");
    let c = Catalog::prepare(&catalog_path, pg_library::versions(), PATH_RULES).unwrap();
    let a = pg_platform::names::normalize_path(&dir.join("a")).unwrap();
    let b = pg_platform::names::normalize_path(&dir.join("b")).unwrap();
    assert!(c.add_root(&a).unwrap());
    assert!(c.add_root(&b).unwrap());
    (dir.clone(), catalog_path, a, b)
}

/// A runner over `catalog_path`, whose drives are asked of `drives`. The thumbnail cache's events are kept by the caller,
/// so the cache's sender lives as long as the runner does.
#[expect(clippy::unwrap_used, reason = "a test's setup: its failure is the test's")]
fn start(dir: &Path, catalog_path: &Path, drives: Fake, features: Features) -> (Runner, Quiet<pg_thumbcache::Event>) {
    let id = Catalog::open(catalog_path, PATH_RULES).unwrap().catalog_id().unwrap();
    let (events, quiet) = Quiet::bus();
    let cache = ThumbCache::open(&dir.join("thumbs.sqlite"), &id, None, pg_render::THUMBS_VERSION, events).unwrap();
    let heights = HeightCell::default();
    heights.set(ThumbHeight::new(64).unwrap());
    let runner = Runner::start_with_drives(
        catalog_path.to_path_buf(),
        Box::leak(Box::new(cache)),
        heights,
        FeatureCell::new(features),
        Arc::new(Bus::new(64)),
        RecordingSink::new(),
        Arc::new(drives),
    )
    .unwrap();
    (runner, quiet)
}

/// Waits (`WAIT` at most) until `done`, and says what it waited for if it never is.
fn wait_for(what: &str, done: impl Fn() -> bool) {
    let until = Instant::now() + WAIT;
    while !done() {
        assert!(Instant::now() < until, "waited too long for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The folders on two drives make two lanes, one for each drive.
#[test]
fn two_drives_make_two_lanes() {
    let (dir, catalog_path, a, b) = two_folders("two-drives");
    let drives = Fake(vec![("a", disk(1)), ("b", disk(2))]);
    let (runner, _quiet) = start(&dir, &catalog_path, drives, Features::ALL);
    runner.scan(vec![a, b]);
    wait_for("two lanes", || runner.lanes() == 2);
    assert!(runner.stop(WAIT));
}

/// Two folders on one drive share one lane: they are one spindle's work, in order.
#[test]
fn folders_on_one_drive_share_a_lane() {
    let (dir, catalog_path, a, b) = two_folders("one-drive");
    let drives = Fake(vec![("a", disk(1)), ("b", disk(1))]);
    let (runner, _quiet) = start(&dir, &catalog_path, drives, Features::ALL);
    runner.scan(vec![a, b]);
    wait_for("one lane", || runner.lanes() == 1);
    // A lane that is made stays: it is not a second one for a later request.
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(runner.lanes(), 1);
    assert!(runner.stop(WAIT));
}

/// A folder whose drive can't be told collapses every folder into one lane: fail closed, as one drive did before lanes.
#[test]
fn a_drive_that_cannot_be_told_collapses_the_lanes() {
    let (dir, catalog_path, a, b) = two_folders("unknown");
    // Only `a` is known: `b`'s drive is an error.
    let drives = Fake(vec![("a", disk(1))]);
    let (runner, _quiet) = start(&dir, &catalog_path, drives, Features::ALL);
    runner.scan(vec![a, b]);
    wait_for("one lane for both", || runner.lanes() == 1);
    assert!(runner.stop(WAIT));
}

/// The escape hatch: with serial drives on, folders on two drives still make one lane.
#[test]
fn serial_drives_make_one_lane_even_on_two_drives() {
    let (dir, catalog_path, a, b) = two_folders("serial");
    let drives = Fake(vec![("a", disk(1)), ("b", disk(2))]);
    let serial = Features { serial_drives: true, ..Features::ALL };
    let (runner, _quiet) = start(&dir, &catalog_path, drives, serial);
    runner.scan(vec![a, b]);
    wait_for("one lane", || runner.lanes() == 1);
    assert!(runner.stop(WAIT));
}

/// A drive with no root left retires its lane: removing its folder from the library leaves the other lane.
#[test]
fn removing_a_drives_last_folder_retires_its_lane() {
    let (dir, catalog_path, a, b) = two_folders("retire");
    let drives = Fake(vec![("a", disk(1)), ("b", disk(2))]);
    let (runner, _quiet) = start(&dir, &catalog_path, drives, Features::ALL);
    runner.scan(vec![a.clone(), b.clone()]);
    wait_for("two lanes", || runner.lanes() == 2);
    let c = Catalog::open(&catalog_path, PATH_RULES).unwrap();
    let id = c.roots().unwrap().into_iter().find(|root| root.path == b).unwrap().id;
    c.remove_root(id).unwrap();
    runner.wake();
    wait_for("the lane of the removed folder to retire", || runner.lanes() == 1);
    assert!(runner.stop(WAIT));
}

/// A stop ends every lane and the coordinator inside the wait, with no round left running.
#[test]
fn a_stop_ends_every_lane_within_the_wait() {
    let (dir, catalog_path, a, b) = two_folders("stop");
    let drives = Fake(vec![("a", disk(1)), ("b", disk(2))]);
    let (runner, _quiet) = start(&dir, &catalog_path, drives, Features::ALL);
    runner.scan(vec![a, b]);
    wait_for("two lanes", || runner.lanes() == 2);
    let began = Instant::now();
    assert!(runner.stop(WAIT), "every thread ends");
    assert!(began.elapsed() < WAIT);
    assert!(runner.idle(), "no lane is in a round after the stop");
}

/// Each lane reports its drive's label with its progress: two drives, two labelled lanes, in the order of their roots.
#[test]
fn each_lane_reports_its_drive_label() {
    let (dir, catalog_path, a, b) = two_folders("labels");
    let drives = Fake(vec![("a", disk(1)), ("b", disk(2))]);
    let (runner, _quiet) = start(&dir, &catalog_path, drives, Features::ALL);
    runner.scan(vec![a, b]);
    wait_for("two lanes", || runner.lanes() == 2);
    wait_for("both lanes to report their labels", || {
        let lanes = runner.lane_progress();
        lanes.len() == 2 && lanes.iter().all(|lane| !lane.label.is_empty())
    });
    let lanes = runner.lane_progress();
    assert_ne!(lanes[0].label, lanes[1].label, "one drive's label is not another's");
    assert!(runner.stop(WAIT));
}

/// A pause of one lane (L7) leaves the other lanes running: only the paused lane reports itself paused, the run's global
/// pause is not set, and a number that no lane has is refused.
#[test]
fn a_per_lane_pause_pauses_only_its_lane() {
    let (dir, catalog_path, a, b) = two_folders("lane-pause");
    let drives = Fake(vec![("a", disk(1)), ("b", disk(2))]);
    let (runner, _quiet) = start(&dir, &catalog_path, drives, Features::ALL);
    runner.scan(vec![a, b]);
    wait_for("two lanes", || runner.lanes() == 2);
    let first = runner.lane_progress()[0].number;
    let second = runner.lane_progress()[1].number;
    assert!(runner.set_lane_paused(first, true));
    wait_for("the first lane to report itself paused", || {
        let lanes = runner.lane_progress();
        lanes.iter().any(|lane| lane.number == first && lane.paused)
    });
    let lanes = runner.lane_progress();
    assert!(lanes.iter().any(|lane| lane.number == second && !lane.paused), "the other lane runs on");
    assert!(!runner.is_paused(), "a lane's pause is not the run's pause");
    assert!(!runner.set_lane_paused(first.wrapping_add(1_000), true), "no such lane");
    assert!(runner.set_lane_paused(first, false));
    assert!(runner.stop(WAIT));
}
