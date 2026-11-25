//! The app's background job: one lane per drive, each a thread that runs the round of passes over its volumes (`lane`,
//! `passes`), and the coordinator that plans the lanes from the drives the library's folders are on (`coordinator`,
//! `lanes`). Folders on two drives are walked and processed at once; folders on one drive go one after the other, since two
//! readers on one spindle fight over its head.
//!
//! A new request (a folder added, a rescan) stops the round in progress at its next file, on the lane of its folder; a new
//! thumbnail height or a feature turned on or off does not stop a round, but the pass it concerns ends after its batch (each
//! watches the value it runs for). A pass commits what it has and the next round starts with the new work. Every pass is
//! resumable from the catalog, so stopping costs nothing but the batch in flight. The notices the lanes share are told
//! through `tally`, which says what all the lanes say together.

use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use crate::{Error, Result};
use pg_catalog::Catalog;
use pg_jobs::NoticeSink;
use pg_jobs::control::Control;
use pg_jobs::events::Bus;
use pg_model::ids::FileId;
use pg_model::job::Change;
use pg_model::progress::{Phase, Progress};
use pg_model::rules::Path as RulePath;
use pg_platform::drive::{Drives, SystemDrives};
use pg_platform::native::PATH_RULES;
use pg_thumbcache::ThumbCache;

use crate::coordinator::Lanes;
use crate::mask::{PassKind, PassMask};
use crate::shared::{FeatureCell, Generation, HeightCell, Pins, Shared};
use crate::tuning::{ProfileStore, TuningLog};
use pg_model::progress::LaneProgress;

/// The settings a job starts with: the counts a developer pins for the run, the passes a round runs, where the lanes keep
/// what they learn of their drives, and who is told when a lane's reader count changes. The handles the job shares (the
/// cells, the change bus, the notices, the drives) are arguments of [`Runner::start`], not settings.
#[derive(Clone, Default)]
pub struct RunnerConfig {
    /// The reads in flight, pinned for the run (a developer's sweep). `None`: each lane tunes its own (`tuning`); a pinned
    /// run tunes nothing and writes no learned profile.
    pub pin_readers: Option<NonZeroUsize>,
    /// The decode threads, pinned for the run. `None` keeps the default (one per logical CPU).
    pub pin_workers: Option<NonZeroUsize>,
    /// The passes each round runs. Every pass by default; a pass left out is skipped in the real round.
    pub passes: PassMask,
    /// Where the lanes keep the reader counts they learned (the app's `drive-profiles.json`). `None`: each run starts from
    /// the drive kinds' priors, and keeps nothing.
    pub profiles: Option<Arc<dyn ProfileStore>>,
    /// Told one line for each change of a lane's reader count (the app logs it, for the dev-mode log view: the library
    /// writes no log of its own).
    pub on_tuning: Option<TuningLog>,
}

impl std::fmt::Debug for RunnerConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunnerConfig")
            .field("pin_readers", &self.pin_readers)
            .field("pin_workers", &self.pin_workers)
            .field("passes", &self.passes)
            .field("profiles", &self.profiles.is_some())
            .field("on_tuning", &self.on_tuning.is_some())
            .finish()
    }
}

/// The background job the module comment describes. It holds the lanes, one per drive the library's folders are on.
pub struct Runner {
    pub(crate) lanes: Arc<Lanes>,
}

impl Runner {
    /// Start the job. It makes a lane for each drive the library's folders are on, as the drives are found out; each lane
    /// begins by importing whatever is pending on its drive, and makes the thumbnails still missing at the height `heights`
    /// holds (none until one is set). `features` says which background work is on (the borders, the duplicates). What it
    /// changes is published on `changes` (`Change`), and what it tells the user goes to `notices`, which is poked whenever a
    /// pass changes the failure ledger (`Control::ledger_changed`). `config` holds the run's settings (`RunnerConfig`). An
    /// error if the job's threads can't be started.
    pub fn start(
        catalog_path: PathBuf,
        thumbs: Arc<ThumbCache>,
        heights: HeightCell,
        features: FeatureCell,
        changes: Arc<Bus<Change>>,
        notices: Arc<dyn NoticeSink>,
        config: RunnerConfig,
    ) -> Result<Runner> {
        Runner::start_with_drives(
            catalog_path,
            thumbs,
            heights,
            features,
            changes,
            notices,
            Arc::new(SystemDrives),
            config,
        )
    }

    /// [`Runner::start`] with the drives the library's roots are on asked of `drives` (the system's, or a test's).
    #[expect(clippy::too_many_arguments, reason = "the job's handles, and its settings: each is a distinct argument")]
    pub fn start_with_drives(
        catalog_path: PathBuf,
        thumbs: Arc<ThumbCache>,
        heights: HeightCell,
        features: FeatureCell,
        changes: Arc<Bus<Change>>,
        notices: Arc<dyn NoticeSink>,
        drives: Arc<dyn Drives + Send + Sync>,
        config: RunnerConfig,
    ) -> Result<Runner> {
        let shared = Shared::new(catalog_path, thumbs, heights.clone(), features.clone(), notices, changes);
        Runner::start_built(shared, heights, features, drives, config)
    }

    /// A runner over the test's hooks (`hooks`), in place before the job starts, so a hook holds even the removal the
    /// library starts with. The change bus and the notices are a test's own.
    #[cfg(test)]
    pub(crate) fn start_hooked(
        catalog_path: PathBuf,
        thumbs: Arc<ThumbCache>,
        heights: HeightCell,
        features: FeatureCell,
        drives: Arc<dyn Drives + Send + Sync>,
        hooks: Arc<crate::shared::TestHooks>,
    ) -> Result<Runner> {
        let mut shared = Shared::new(
            catalog_path,
            thumbs,
            heights.clone(),
            features.clone(),
            pg_jobs_testkit::RecordingSink::new(),
            Arc::new(Bus::new(64)),
        );
        shared.hooks = hooks;
        Runner::start_built(shared, heights, features, drives, RunnerConfig::default())
    }

    /// The job over `shared`, started: its lanes, its coordinator and its removals thread. The settings are in place before
    /// the lanes start, so the first round already runs with them.
    fn start_built(
        shared: Shared,
        heights: HeightCell,
        features: FeatureCell,
        drives: Arc<dyn Drives + Send + Sync>,
        config: RunnerConfig,
    ) -> Result<Runner> {
        shared.passes.set(config.passes);
        let mut shared = shared;
        let pins = Pins { readers: config.pin_readers, workers: config.pin_workers };
        shared.options = crate::inner::pinned_options(pins)
            .map_err(|refused| Error::step(format!("the pinned counts {pins:?} are refused"), refused))?;
        shared.pinned = pins.readers.is_some();
        shared.profiles = config.profiles;
        shared.on_tuning = config.on_tuning;
        let shared = Arc::new(shared);
        let root = Arc::new(Control::new());
        let announce = shared.changes.clone();
        root.on_change(move || announce.publish(Change::Progress));
        // The inbox reads the ledger again when a pass changed it, within the pass, not only at the round's end.
        let ledger = shared.notices.clone();
        root.on_ledger_change(move || ledger.poke());
        let lanes = Lanes::start(shared, root, drives)?;
        // A new height or a change of features wakes every lane; the passes they concern end after their batch, and the next
        // round reads the new values. Weak references: the cells (held by the library) must not keep the job alive.
        let weak = Arc::downgrade(&lanes);
        heights.listen(move || {
            if let Some(lanes) = weak.upgrade() {
                lanes.wake_all(false);
            }
        });
        let weak = Arc::downgrade(&lanes);
        features.listen(move || {
            if let Some(lanes) = weak.upgrade() {
                lanes.wake_all(false);
            }
        });
        Ok(Runner { lanes })
    }

    /// Stop the job after the file (or the batch) in progress, as a new request does, and wait for its threads to end, for at
    /// most `timeout`: their connections close with them. `false` if one is still running then; it ends with the process,
    /// and the WAL it left is merged at the next open.
    pub fn stop(&self, timeout: Duration) -> bool {
        self.lanes.stop(timeout)
    }

    /// Scan these folders (normalized), then import and thumbnail what they hold.
    pub fn scan(&self, roots: Vec<RulePath>) {
        self.lanes.request_scans(roots);
    }

    /// A library folder was removed: the scans asked for under `root` are dropped, and one walking a folder under it is
    /// woken. That walk writes nothing more: the removal took its lease (`Catalog::remove_root`), so its next write
    /// ends it as `ScanError::Removed`, and it isn't asked for again, as the folder is no longer in the library.
    pub fn stop_scans_under(&self, root: &RulePath) {
        self.lanes.stop_scans_under(root);
    }

    /// A removal began (`Catalog::begin_removal`): its lanes are parked on the folder's volumes now, so no lane starts a
    /// round on them before the removal hides the folder's files (section 1, D1). Does not wait.
    pub fn park_removal(&self, path: &RulePath, volumes: &pg_model::removal::Volumes) {
        self.lanes.park_removal(path, volumes);
    }

    /// Save cropped copies of these files' borders into `folder` (`pg_crop::save`), before anything else.
    pub fn save_crops(&self, ids: Vec<FileId>, folder: PathBuf) {
        self.lanes.request_crop(ids, folder);
    }

    /// Cut the borders off the originals of these files, in place (`pg_crop::cut`), after the copies asked for. Nothing
    /// is queued unless `confirmed`: the originals change, and only the user's confirmation says they may.
    pub fn cut_borders(&self, ids: Vec<FileId>, confirmed: bool) {
        if confirmed {
            self.lanes.request_cut(ids);
        }
    }

    /// A folder was taken out of the library (`Catalog::begin_removal`): its removal is finished by the removal job now,
    /// and not only at its next recheck.
    pub fn wake_removals(&self) {
        self.lanes.wake_removals();
    }

    /// The catalog has new work outside a scan (a file turned in place needs its analysis redone): every lane starts a round,
    /// and the running ones stop at their next file.
    pub fn wake(&self) {
        self.lanes.wake_all(true);
    }

    /// Files were turned in place: only the lanes of their volumes start again, so the other drives' rounds are not
    /// interrupted. Nothing is done for no files.
    pub fn wake_files(&self, ids: Vec<FileId>) {
        self.lanes.wake_files(ids);
    }

    /// The lanes the job has now: one per physical drive its roots are on (`coordinator`), or one when they collapse.
    pub fn lanes(&self) -> usize {
        self.lanes.all().len()
    }

    /// Whether no lane is in a round now (for the measuring harness: it waits for the job to be idle).
    pub fn idle(&self) -> bool {
        self.lanes.all().iter().all(|lane| !lane.inner.is_active())
    }

    /// Whether the job has nothing left to do, and nothing that is owed is still running: no lane is in a round, no crop or cut
    /// is queued or running, no write is left, and no removal is unfinished. The folder walks count only while the scan pass
    /// runs (`set_passes`): a walk asked for while it is left out waits for the next run that includes it, as the work of any
    /// pass left out does, and is not a reason to wait here. A root with no lane (its drive is not there) keeps the job busy
    /// while its walk is asked for, so a run waiting for it times out rather than finishing early. `false` when the catalog
    /// can't be read.
    pub fn quiescent(&self) -> bool {
        let scans = self.passes().contains(PassKind::Scans);
        let lanes_quiet = self.lanes.all().iter().all(|lane| {
            !lane.inner.is_active() && (!scans || (lane.inner.scans().is_empty() && lane.inner.busy().is_empty()))
        });
        if !lanes_quiet || !self.lanes.unrouted(scans) {
            return false;
        }
        let (crop, _) = self.lanes.crop_snapshot();
        if crop.phase != Phase::Idle {
            return false;
        }
        match Catalog::open(&self.lanes.shared.catalog_path, PATH_RULES) {
            Ok(catalog) => {
                catalog.left_to_write().is_ok_and(|left| left == 0)
                    && catalog.removal_markers().is_ok_and(|markers| markers.is_empty())
            }
            Err(_) => false,
        }
    }

    /// The passes each round runs (`RunnerConfig::passes`, or the last `set_passes`).
    pub fn passes(&self) -> PassMask {
        self.lanes.shared.passes.get()
    }

    /// Sets the passes each round runs. A round in progress keeps the set it began with; the next round uses this one. The
    /// passes left out are not run, and what they owe stays owed.
    pub fn set_passes(&self, passes: PassMask) {
        self.lanes.shared.passes.set(passes);
    }

    /// Pauses or resumes the whole run (every lane).
    pub fn set_paused(&self, paused: bool) {
        self.lanes.root.set_paused(paused);
    }

    /// Whether the whole run is paused.
    pub fn is_paused(&self) -> bool {
        self.lanes.root.is_paused()
    }

    /// Every lane's progress, in the lanes' order, each with its drive's label (`LaneProgress`). The library's one progress
    /// (`progress`) is the first of them that works.
    pub fn lane_progress(&self) -> Vec<LaneProgress> {
        self.lanes.progress_lines()
    }

    /// The crop thread's progress and its time since work (`LaneProgress::since_work`): the crop is a row of its own.
    pub fn crop_progress(&self) -> (Progress, Option<Duration>) {
        self.lanes.crop_snapshot()
    }

    /// Pauses or resumes one lane (by its `LaneProgress::number`), leaving the others running. `false` when no lane has
    /// that number. The run's global pause (`set_paused`) still holds every lane.
    pub fn set_lane_paused(&self, number: usize, paused: bool) -> bool {
        self.lanes.set_lane_paused(number, paused)
    }

    /// The progress of the first job that is busy (the crop thread's, then the lanes' in order), for the app's one line.
    /// Idle when none is.
    pub fn progress(&self) -> Progress {
        self.lanes
            .controls()
            .iter()
            .map(|control| control.snapshot())
            .find(|progress| progress.phase != Phase::Idle)
            .unwrap_or_else(|| self.lanes.root.snapshot())
    }

    /// The generation of the library's state: it changes when the library's contents do.
    pub fn generation(&self) -> Generation {
        self.lanes.shared.generation()
    }

    /// Rounds finished so far, for the app to count them as stages: every lane's.
    pub fn rounds(&self) -> u64 {
        self.lanes.shared.rounds.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::atomic::AtomicU64;

    use pg_catalog::Catalog;
    use pg_jobs::pipeline::Options;
    use pg_jobs::schedule::{Pass, Step};
    use pg_jobs_testkit::{Call, Quiet, RecordingSink};
    use pg_model::features::Features;
    use pg_model::notice::{FilePass, Key, Notice, Op, ScanNote, Trigger};
    use pg_model::size::ThumbHeight;
    use pg_platform::native::PATH_RULES;
    use pg_platform::priority::Priority;
    use pg_thumbs::LedgerOutcome;

    use super::*;
    use crate::inner::Inner;
    use crate::passes::{Borders, Cx, Hashes, Pairing, Scans, Thumbs};

    /// A height cell set to 64 pixels, as the grid asks at the start.
    fn height_64() -> HeightCell {
        let heights = HeightCell::default();
        heights.set(ThumbHeight::new(64).expect("a height in range"));
        heights
    }

    /// A height is held before anything listens, and each change to a different height wakes the job once; the same height
    /// again wakes nothing.
    #[test]
    fn a_height_is_held_and_each_change_wakes_the_job_once() {
        let heights = HeightCell::default();
        assert_eq!(heights.get(), None, "no height until the grid asks");
        let woke = Arc::new(AtomicU64::new(0));
        let counter = woke.clone();
        heights.listen(move || {
            counter.fetch_add(1, Ordering::Relaxed);
        });
        let at = |px| ThumbHeight::new(px).expect("a height in range");
        heights.set(at(220));
        heights.set(at(220));
        heights.set(at(440));
        assert_eq!(heights.get(), Some(at(440)));
        assert_eq!(woke.load(Ordering::Relaxed), 2);
    }

    /// The features are held as they were set, and each change to other features wakes the job once; the same features
    /// again wake nothing.
    #[test]
    fn features_are_held_and_each_change_wakes_the_job_once() {
        let features = FeatureCell::new(Features::ALL);
        let woke = Arc::new(AtomicU64::new(0));
        let counter = woke.clone();
        features.listen(move || {
            counter.fetch_add(1, Ordering::Relaxed);
        });
        let no_duplicates = Features { borders: true, duplicates: false };
        features.set(Features::ALL);
        features.set(no_duplicates);
        features.set(no_duplicates);
        assert_eq!(features.get(), no_duplicates);
        assert_eq!(woke.load(Ordering::Relaxed), 1);
    }

    /// A library folder `images` in a run folder, holding `names` (small PNGs), scanned and imported into the catalog
    /// beside it: the run folder, the catalog's path, and the library folder.
    fn library(prefix: &str, names: &[&str]) -> (PathBuf, PathBuf, RulePath) {
        library_sized(prefix, names, 8)
    }

    /// [`library`], with square pictures of `side` pixels (the border detector looks at those of 64 or more).
    fn library_sized(prefix: &str, names: &[&str], side: u32) -> (PathBuf, PathBuf, RulePath) {
        let dir = pg_testkit::run_folder_with("scratch_library", prefix, &["images"]).unwrap();
        // A pattern, not a flat colour: a flat picture has too little PDQ quality to owe its pairs.
        let pixels: Vec<u8> =
            (0..side * side * 3).map(|i| u8::try_from(i.wrapping_mul(2_654_435_761) >> 24).unwrap()).collect();
        for name in names {
            std::fs::write(dir.join("images").join(name), pg_codec::fixtures::png_rgb(side, side, &pixels).unwrap())
                .unwrap();
        }
        let catalog_path = dir.join("catalog.sqlite");
        let mut c = Catalog::prepare(&catalog_path, crate::versions(), PATH_RULES).unwrap();
        let root = pg_platform::names::normalize_path(&dir.join("images")).unwrap();
        assert!(c.add_root(&root).unwrap());
        pg_scan::scan(&mut c, &root, &Control::new()).unwrap();
        pg_import::import_while(
            &mut c,
            &Options::new(Priority::Normal),
            &pg_catalog::VolumeSet::All,
            &|| true,
            &Control::new(),
        )
        .unwrap();
        (dir, catalog_path, root)
    }

    /// With a feature off, its passes do nothing and what they owe stays owed: the Hashes and Pairing passes while
    /// duplicates is off, the Borders pass while borders is off. Turned back on, the Hashes pass makes the hashes owed.
    #[test]
    fn a_pass_whose_feature_is_off_does_nothing_and_its_work_stays_owed() {
        let (dir, catalog_path, _root) = library("runner-off", &["a.png"]);
        let (cache, _quiet) = thumbs(&dir, &catalog_path, None);
        let sink = RecordingSink::new();
        let inner = job(&catalog_path, cache.clone(), sink.clone());
        inner.shared.features.set(Features { borders: false, duplicates: false });
        let due = pg_catalog::fixture::state::borders_due(&Catalog::open(&catalog_path, PATH_RULES).unwrap(), "a.png")
            .unwrap();
        assert_eq!(run(&mut Hashes, &inner), Step::Done);
        assert_eq!(run(&mut Pairing, &inner), Step::Done);
        assert_eq!(run(&mut Borders, &inner), Step::Done);
        let c = Catalog::open(&catalog_path, PATH_RULES).unwrap();
        assert_eq!(c.owed_hashes().unwrap(), 1, "the hashes stay owed while duplicates is off");
        assert_eq!(pg_catalog::fixture::state::borders_due(&c, "a.png").unwrap(), due, "the borders are not touched");

        inner.shared.features.set(Features { borders: false, duplicates: true });
        assert_eq!(run(&mut Hashes, &inner), Step::Done);
        assert_eq!(c.owed_hashes().unwrap(), 0, "turned on, the pass makes them");
    }

    /// One round makes the thumbnails, then the placeholders, then the hashes, then the pairs, then the borders: each pass ends its phase
    /// (`Finished`) in that order. A picture imported without its hashes owes all four, at a 300-pixel size the borders
    /// look at.
    #[test]
    fn a_round_makes_the_thumbnails_then_the_hashes_then_the_pairs_then_the_borders() {
        let (dir, catalog_path, _root) = library_sized("runner-order", &["a.png"], 300);
        let (cache, _quiet) = thumbs(&dir, &catalog_path, None);
        let sink = RecordingSink::new();
        let inner = job(&catalog_path, cache.clone(), sink.clone());
        let ended = inner.ctl.subscribe();
        inner.round(&mut None).unwrap();
        assert_eq!(finished_order(ended.take().items), ["thumbs", "placeholders", "hashes", "pairs", "borders"]);
    }

    /// The passes that ended, in the order they ended, among the four the round's order is about (a pass that ends with
    /// nothing to do ends too, and says so as the others do).
    fn finished_order(events: Vec<pg_jobs::control::Event>) -> Vec<&'static str> {
        events
            .into_iter()
            .filter_map(|event| match event {
                pg_jobs::control::Event::Finished(progress) => match progress.phase {
                    pg_model::progress::Phase::Thumbs { .. } => Some("thumbs"),
                    pg_model::progress::Phase::Placeholders => Some("placeholders"),
                    pg_model::progress::Phase::Hashes => Some("hashes"),
                    pg_model::progress::Phase::Pairing => Some("pairs"),
                    pg_model::progress::Phase::Borders => Some("borders"),
                    _ => None,
                },
                pg_jobs::control::Event::Progress(_) => None,
            })
            .collect()
    }

    /// A lane's round ends with a PASSIVE checkpoint on the catalog it used, so the round's writes leave the WAL (section 11).
    #[test]
    fn a_lane_round_checkpoints_the_wal_at_its_end() {
        let (dir, catalog_path, _root) = library_sized("runner-checkpoint", &["a.png"], 300);
        let (cache, _quiet) = thumbs(&dir, &catalog_path, None);
        let sink = RecordingSink::new();
        let inner = job(&catalog_path, cache.clone(), sink.clone());
        let mut conn = None;
        inner.round(&mut conn).unwrap();
        assert!(conn.as_ref().expect("the round keeps its catalog").passive_checkpoints() >= 1);
    }

    /// A thumbnail cache that refuses its batch is said (`CacheNotStored`), and the round goes on: the hashes, the pairs
    /// and the borders still run after the thumbnails. (Before this, the error ended the round, and they never ran.)
    #[test]
    fn a_thumbnail_cache_error_is_said_and_the_round_goes_on() {
        let (dir, catalog_path, _root) = library_sized("runner-cache-error", &["a.png"], 300);
        let (cache, _quiet) = thumbs(&dir, &catalog_path, None);
        pg_thumbcache::fault::refuse_stores(&dir.join("thumbs.sqlite")).unwrap();
        let sink = RecordingSink::new();
        let inner = job(&catalog_path, cache.clone(), sink.clone());
        let ended = inner.ctl.subscribe();
        inner.round(&mut None).unwrap();
        let posted = sink.posted();
        assert!(
            posted.iter().any(|n| matches!(n, Notice::CacheNotStored { error } if error.contains("refuses"))),
            "{posted:?}"
        );
        assert_eq!(finished_order(ended.take().items), ["thumbs", "placeholders", "hashes", "pairs", "borders"]);
    }

    /// The thumbnail cache beside the catalog, holding at most `cap` bytes (`None`: the default), leaked as the job's
    /// is, with the watch on its events.
    fn thumbs(dir: &Path, catalog_path: &Path, cap: Option<i64>) -> (Arc<ThumbCache>, Quiet<pg_thumbcache::Event>) {
        let id = Catalog::open(catalog_path, PATH_RULES).unwrap().catalog_id().unwrap();
        let (events, quiet) = Quiet::bus();
        let cap = cap.and_then(pg_model::cache::ThumbCap::from_bytes);
        let cache =
            ThumbCache::open(&dir.join("thumbs.sqlite"), &id, cap, pg_model::versions::THUMBS_VERSION, events).unwrap();
        (Arc::new(cache), quiet)
    }

    /// The job's state without its thread: the test runs its passes one by one.
    fn job(catalog_path: &Path, thumbs: Arc<ThumbCache>, sink: Arc<RecordingSink>) -> Inner {
        let shared = Shared::new(
            catalog_path.to_path_buf(),
            thumbs,
            height_64(),
            FeatureCell::new(Features::ALL),
            sink,
            Arc::new(Bus::new(16)),
        );
        let budget = Arc::new(pg_jobs::governor::Budget::new(48 << 20));
        Inner::new(Arc::new(shared), 0, Arc::new(Control::new()), budget)
    }

    /// Run `pass` once on a connection of its own, as a round does.
    fn run(pass: &mut dyn for<'a> Pass<Cx<'a>>, inner: &Inner) -> Step {
        let mut catalog = Catalog::open(&inner.shared.catalog_path, PATH_RULES).unwrap();
        pass.run(&mut Cx { inner, catalog: &mut catalog }).unwrap()
    }

    /// A file that can't be read (here, gone since the import) is said, not recorded in the ledger; once nothing is
    /// left to read, the next complete pass (its early return) clears what was said.
    #[test]
    fn an_offline_file_is_said_not_recorded_and_a_complete_pass_clears_it() {
        let (dir, catalog_path, root) = library("runner-offline", &["a.png", "b.png"]);
        let (cache, _quiet) = thumbs(&dir, &catalog_path, None);
        let sink = RecordingSink::new();
        let inner = job(&catalog_path, cache.clone(), sink.clone());
        std::fs::remove_file(dir.join("images").join("b.png")).unwrap();
        run(&mut Thumbs, &inner);
        assert_eq!(sink.posted(), [Notice::UnreadableFiles { pass: FilePass::Thumbs, count: 1, drives: Vec::new() }]);
        let c = Catalog::open(&catalog_path, PATH_RULES).unwrap();
        assert!(c.failed_files(FilePass::Thumbs, 10).unwrap().is_empty(), "an unreadable file is in no ledger");
        // The rescan finds it missing: the next pass has nothing to make, and is complete.
        let mut catalog = Catalog::open(&catalog_path, PATH_RULES).unwrap();
        catalog.add_root(&root).unwrap();
        pg_scan::scan(&mut catalog, &root, &Control::new()).unwrap();
        sink.take();
        run(&mut Thumbs, &inner);
        assert!(sink.posted().is_empty(), "{:?}", sink.calls());
        assert!(sink.calls().contains(&Call::Resolve(Trigger::Pass(FilePass::Thumbs))), "{:?}", sink.calls());
    }

    /// A cache too small for one thumbnail is said; a complete pass (here, once its file left the library) clears it.
    #[test]
    fn a_full_cache_is_said_and_a_complete_pass_clears_it() {
        let (dir, catalog_path, root) = library("runner-full", &["a.png"]);
        let (cache, _quiet) = thumbs(&dir, &catalog_path, Some(1));
        let sink = RecordingSink::new();
        let inner = job(&catalog_path, cache.clone(), sink.clone());
        run(&mut Thumbs, &inner);
        assert_eq!(
            sink.posted(),
            [Notice::CacheFull { height: ThumbHeight::new(64).expect("a height in range"), made: 0 }]
        );
        std::fs::remove_file(dir.join("images").join("a.png")).unwrap();
        let mut catalog = Catalog::open(&catalog_path, PATH_RULES).unwrap();
        catalog.add_root(&root).unwrap();
        pg_scan::scan(&mut catalog, &root, &Control::new()).unwrap();
        sink.take();
        run(&mut Thumbs, &inner);
        assert!(sink.calls().contains(&Call::Resolve(Trigger::Pass(FilePass::Thumbs))), "{:?}", sink.calls());
    }

    /// A round that fails (its catalog can't be opened) is said; the next round that runs clears it.
    #[test]
    fn a_failed_round_is_said_and_the_next_round_clears_it() {
        let (dir, catalog_path, _root) = library("runner-round", &[]);
        let (cache, _quiet) = thumbs(&dir, &catalog_path, None);
        let aside = dir.join("aside.sqlite");
        std::fs::rename(&catalog_path, &aside).unwrap();
        let sink = RecordingSink::new();
        let runner = Runner::start(
            catalog_path.clone(),
            cache,
            height_64(),
            FeatureCell::new(Features::ALL),
            Arc::new(Bus::new(16)),
            sink.clone(),
            RunnerConfig::default(),
        )
        .unwrap();
        pg_testkit::wait_until(
            Duration::from_secs(30),
            || "the round's failure",
            || sink.posted().iter().any(|n| matches!(n, Notice::RoundFailed { .. })),
        );
        std::fs::rename(&aside, &catalog_path).unwrap();
        runner.wake();
        pg_testkit::wait_until(
            Duration::from_secs(30),
            || "the failure cleared",
            || sink.calls().contains(&Call::Resolve(Trigger::Op(Op::Round))),
        );
        assert!(runner.stop(Duration::from_secs(30)));
    }

    /// The scan notes split by what they describe (`ScanNote::lifecycle`): a complete walk of the library folder
    /// replaces the folder's conditions with what it found (here, a second path to a file: a hard link); a walk of a
    /// folder inside it says nothing of them.
    #[test]
    fn only_a_walk_of_the_whole_library_folder_replaces_its_conditions() {
        let (dir, catalog_path, root) = library("runner-notes", &["a.png"]);
        let images = dir.join("images");
        std::fs::create_dir_all(images.join("sub")).unwrap();
        if std::fs::hard_link(images.join("a.png"), images.join("sub").join("b.png")).is_err() {
            pg_platform::testing::skip("this file system makes no hard links");
            return;
        }
        let (cache, _quiet) = thumbs(&dir, &catalog_path, None);
        let sink = RecordingSink::new();
        let inner = job(&catalog_path, cache.clone(), sink.clone());
        let sub = pg_platform::names::normalize_path(&images.join("sub")).unwrap();
        inner.scans().extend([root.clone(), sub]);
        assert_eq!(run(&mut Scans, &inner), Step::Done);
        let duplicates = Notice::ScanNote { root: root.clone(), note: ScanNote::Duplicates, count: 1 };
        assert_eq!(sink.take(), [Call::Replace(Key::all("scan-note", Some(&root)).to_buf(), vec![duplicates])]);
    }

    /// A walk refused by another's lease posts nothing, waits apart, and runs once it is put back after the lease ends.
    #[test]
    fn a_walk_refused_by_a_lease_is_tried_again_after_it() {
        let (dir, catalog_path, root) = library("runner-busy", &["a.png"]);
        let (cache, _quiet) = thumbs(&dir, &catalog_path, None);
        let sink = RecordingSink::new();
        let inner = job(&catalog_path, cache.clone(), sink.clone());
        let mut holder = Catalog::open(&catalog_path, PATH_RULES).unwrap();
        let lease = holder.begin_scan(&root).unwrap();
        inner.scans().push_back(root.clone());
        assert_eq!(run(&mut Scans, &inner), Step::Done);
        assert_eq!(*inner.busy(), std::slice::from_ref(&root));
        assert!(sink.calls().is_empty(), "{:?}", sink.calls());
        drop(lease);
        inner.requeue_busy();
        assert!(inner.busy().is_empty());
        assert_eq!(run(&mut Scans, &inner), Step::Done);
        assert_eq!(sink.take(), [Call::Replace(Key::all("scan-note", Some(&root)).to_buf(), Vec::new())]);
    }

    /// A library folder the scan refuses (here, gone since it was added: its facts can't be read) is the folder
    /// ledger's, not a failed scan: nothing is posted, and the refusal is a row of the ledger.
    #[test]
    fn a_refused_folder_is_in_the_folder_ledger_not_a_failed_scan() {
        let (dir, catalog_path, root) = library("runner-refused", &["a.png"]);
        let (cache, _quiet) = thumbs(&dir, &catalog_path, None);
        let sink = RecordingSink::new();
        let inner = job(&catalog_path, cache.clone(), sink.clone());
        std::fs::remove_dir_all(dir.join("images")).unwrap();
        inner.scans().push_back(root.clone());
        assert_eq!(run(&mut Scans, &inner), Step::Done);
        assert!(sink.posted().is_empty(), "{:?}", sink.calls());
        let c = Catalog::open(&catalog_path, PATH_RULES).unwrap();
        let failed = c.failed_folders(10).unwrap();
        assert_eq!(
            failed.iter().map(|f| (&f.path, f.failure)).collect::<Vec<_>>(),
            [(&root, pg_model::ledger::FolderFailure::Refused)]
        );
    }

    /// The thumbnail pass's ledger outcome: a failure is said, a write clears what was said, and nothing written says
    /// nothing.
    #[test]
    fn a_ledger_outcome_is_said_cleared_or_left() {
        let (dir, catalog_path, _root) = library("runner-ledger", &[]);
        let (cache, _quiet) = thumbs(&dir, &catalog_path, None);
        let sink = RecordingSink::new();
        let inner = job(&catalog_path, cache.clone(), sink.clone());
        inner.ledger_written(LedgerOutcome::Untouched);
        assert!(sink.take().is_empty());
        inner.ledger_written(LedgerOutcome::Failed("disk full".into()));
        assert_eq!(sink.take(), [Call::Post(Notice::LedgerNotWritten { error: "disk full".into() })]);
        inner.ledger_written(LedgerOutcome::Written);
        assert_eq!(sink.take(), [Call::Resolve(Trigger::Op(Op::LedgerWrite(FilePass::Thumbs)))]);
    }

    /// The job pokes its sink when a pass says the ledger changed (`Control::ledger_changed`).
    #[test]
    fn a_ledger_change_pokes_the_sink() {
        let (dir, catalog_path, _root) = library("runner-poke", &[]);
        let (cache, _quiet) = thumbs(&dir, &catalog_path, None);
        let sink = RecordingSink::new();
        let runner = Runner::start(
            catalog_path.clone(),
            cache,
            height_64(),
            FeatureCell::new(Features::ALL),
            Arc::new(Bus::new(16)),
            sink.clone(),
            RunnerConfig::default(),
        )
        .unwrap();
        let before = sink.pokes();
        runner.lanes.root.ledger_changed();
        assert_eq!(sink.pokes(), before + 1);
        assert!(runner.stop(Duration::from_secs(30)));
    }

    /// A cut is queued only once confirmed: an unconfirmed one queues nothing. A confirmed one runs its pass, which says
    /// what became of the file (here, a file with no borders to cut: skipped).
    #[test]
    fn a_cut_is_queued_only_when_confirmed_and_is_told() {
        let (dir, catalog_path, _root) = library("runner-cut", &["a.png"]);
        let (cache, _quiet) = thumbs(&dir, &catalog_path, None);
        let sink = RecordingSink::new();
        let runner = Runner::start(
            catalog_path.clone(),
            cache,
            height_64(),
            FeatureCell::new(Features::ALL),
            Arc::new(Bus::new(16)),
            sink.clone(),
            RunnerConfig::default(),
        )
        .unwrap();
        let c = Catalog::open(&catalog_path, PATH_RULES).unwrap();
        let id = pg_catalog::fixture::state::present_files(&c).unwrap()[0].0;
        runner.cut_borders(vec![id], false);
        assert!(runner.lanes.cuts_queued() == 0, "an unconfirmed cut queues nothing");
        runner.cut_borders(vec![id], true);
        pg_testkit::wait_until(
            Duration::from_secs(30),
            || "the cut's notice",
            || sink.posted().iter().any(|n| matches!(n, Notice::BordersCut { .. })),
        );
        let posted = sink.posted();
        let told = Notice::BordersCut { saved: 0, skipped: 1, unsupported: 0, failed: 0, failures: Vec::new() };
        assert!(posted.contains(&told), "{posted:?}");
        assert!(runner.stop(Duration::from_secs(30)));
    }
}
