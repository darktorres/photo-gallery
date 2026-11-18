//! The job's coordinator: what the lanes share, and the threads that sit beside them.
//!
//! The coordinator thread asks each library root's drive and volume (`probe`, on a probe thread of its own, with a deadline:
//! a dead drive can't be cancelled, so a probe that runs past its deadline makes its root unknown, and it is left running).
//! It plans the lanes from the answers (`lanes::plan`), starts and retires lanes to match, gives each its volumes, and routes
//! the folders asked for to the lane of their root. Nothing it does waits for a lane's round: a lane is asked to stop only
//! when it has work routed to it, or its volumes changed.
//!
//! The crop thread saves cropped copies and cuts borders (`pg_crop`). It works on the files' volumes, so it holds the lanes
//! of those volumes first: each is asked to stop, and the crop waits until its round has ended, so no lane reads a file
//! while it is cut.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError, Weak};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use pg_catalog::{Catalog, VolumeSet};
use pg_jobs::control::{Control, Hold};
use pg_jobs::governor::Governor;
use pg_jobs::panics;
use pg_jobs::pipeline::Options;
use pg_jobs::service::{Service, Waker, Woke};
use pg_model::ids::FileId;
use pg_model::notice::{ERROR_CAP, Notice, tidy};
use pg_model::rules::{Path as RulePath, PathRules};
use pg_platform::drive::{Drives, unplugged};
use pg_platform::native::PATH_RULES;
use pg_platform::priority::Priority;

use crate::lane::Inner;
use crate::lanes::{self, Place};
use crate::passes::{count, cut_done, error_text, file_failure};
use crate::runner::{LaneProgress, Shared};

/// How long a root's probe may run before its root is taken as unknown: the lanes collapse to one until it answers.
const PROBE_DEADLINE: Duration = Duration::from_secs(5);
/// How long a root's place is kept before it is probed again: an unplugged drive is found back this soon.
const PLACE_TTL: Duration = Duration::from_secs(15);
/// How long a retired lane gets to end its round.
const RETIRE_WAIT: Duration = Duration::from_secs(10);
/// The tally's key for the coordinator's own failures (a lane's number is never this).
const COORDINATOR: usize = usize::MAX;
/// The share of the machine's physical memory the lanes may hold for their file buffers together: a quarter.
const MEMORY_SHARE: u64 = 4;

/// A lane, as the coordinator keeps it: its state and its thread, and the library roots it walks.
#[derive(Clone)]
pub(crate) struct Lane {
    pub(crate) inner: Arc<Inner>,
    service: Arc<Service>,
    roots: Vec<RulePath>,
}

/// A root's probe: its answer, and when it came, and when the probe running now began.
#[derive(Default)]
struct Probe {
    answer: Option<(Place, Instant)>,
    running: Option<Instant>,
}

impl Probe {
    /// The root's place now: its answer, or unknown once a probe has run past its deadline. `None` before any answer.
    fn place(&self, now: Instant) -> Option<Place> {
        if self.running.is_some_and(|since| now.saturating_duration_since(since) >= PROBE_DEADLINE) {
            return Some(Place::Unknown);
        }
        self.answer.as_ref().map(|(place, _)| place.clone())
    }

    /// Whether a probe is due now: none is running, and the answer (if any) is older than its life.
    fn wanted(&self, now: Instant) -> bool {
        self.running.is_none()
            && self.answer.as_ref().is_none_or(|(_, at)| now.saturating_duration_since(*at) >= PLACE_TTL)
    }

    /// When the coordinator must look at this probe again: a running probe's deadline (if it is still ahead), or an
    /// answer's expiry. A deadline already past is not a time to wake for: the probe's end wakes the coordinator.
    fn due(&self, now: Instant) -> Option<Instant> {
        match (self.running, &self.answer) {
            (Some(since), _) => {
                let deadline = since + PROBE_DEADLINE;
                (deadline > now).then_some(deadline)
            }
            (None, Some((_, at))) => Some(*at + PLACE_TTL),
            (None, None) => None,
        }
    }
}

/// The coordinator's registry, under one lock.
#[derive(Default)]
struct Registry {
    lanes: Vec<Lane>,
    /// The number the next lane gets. Numbers are not reused: the tally keys its reports by them.
    next_number: usize,
    /// The folders asked for, not yet given to a lane (their root has none yet, or the request is new).
    scans: VecDeque<RulePath>,
    /// The crops asked for: the files, and the folder.
    crops: VecDeque<(Vec<FileId>, PathBuf)>,
    /// The cuts asked for: the files. Queued only once confirmed (`Runner::cut_borders`).
    cuts: VecDeque<Vec<FileId>>,
    /// Each library root's probe, keyed by the root as it is displayed.
    probes: BTreeMap<String, Probe>,
}

/// The volume of `root` among `volumes`, if it has one.
fn volume_of(volumes: &[(RulePath, i64)], root: &RulePath) -> Option<i64> {
    volumes.iter().find(|(path, _)| path == root).map(|(_, volume)| *volume)
}

/// The job's lanes, and the coordinator and crop threads beside them.
pub(crate) struct Lanes {
    /// The run's root control: its global pause, and its stop reaches every lane and the crop.
    pub(crate) root: Arc<Control>,
    /// The crop thread's control: a child of the root, shown as a job of its own.
    crop_ctl: Arc<Control>,
    pub(crate) shared: Arc<Shared>,
    /// Splits the file-buffer budget among the lanes (each lane's [`Inner::options`] carries its share).
    governor: Governor,
    /// Where the roots' drives are asked (the system's, or a test's).
    drives: Arc<dyn Drives + Send + Sync>,
    registry: Mutex<Registry>,
    coordinator: OnceLock<Arc<Service>>,
    cropper: OnceLock<Arc<Service>>,
}

/// One crop or cut, as the crop thread takes it from the queues.
enum Job {
    Save(Vec<FileId>, PathBuf),
    Cut(Vec<FileId>),
}

/// How a crop or cut ended: finished, or stopped with its rest to do.
enum Done {
    Finished,
    Stopped(Job),
}

impl Lanes {
    /// Starts the coordinator and the crop thread. No lane yet: the coordinator makes them when it has the roots' places.
    pub(crate) fn start(
        shared: Arc<Shared>,
        root: Arc<Control>,
        drives: Arc<dyn Drives + Send + Sync>,
    ) -> Result<Arc<Lanes>> {
        let crop_ctl = root.lane();
        // The lanes' CPU is shared: their workers take permits, so two drives' decoders share the cores.
        let cpus = std::thread::available_parallelism().map_or(4, |n| n.get());
        root.share_cpu(cpus);
        // The lanes together hold a quarter of the machine's memory for file buffers, each up to the measured knee.
        let cap = pg_platform::memory::physical_bytes()
            .map(|bytes| usize::try_from(bytes / MEMORY_SHARE).unwrap_or(usize::MAX));
        let governor = Governor::new(Options::budget(8, cpus), cap);
        let lanes = Arc::new(Lanes {
            root,
            crop_ctl,
            shared,
            governor,
            drives,
            registry: Mutex::new(Registry::default()),
            coordinator: OnceLock::new(),
            cropper: OnceLock::new(),
        });
        let weak = Arc::downgrade(&lanes);
        let coordinator = Service::start("lane coordinator", move |waker| coordinate(&weak, waker))
            .context("start the lane coordinator")?;
        let _ = lanes.coordinator.set(Arc::new(coordinator));
        let weak = Arc::downgrade(&lanes);
        let cropper = Service::start("crop job", move |waker| crop(&weak, waker)).context("start the crop job")?;
        let _ = lanes.cropper.set(Arc::new(cropper));
        Ok(lanes)
    }

    fn registry(&self) -> MutexGuard<'_, Registry> {
        self.registry.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The lanes now, in order.
    pub(crate) fn all(&self) -> Vec<Lane> {
        self.registry().lanes.clone()
    }

    /// Each lane's progress, with its drive's label: the probe's answer for the lane's first root.
    pub(crate) fn progress_lines(&self) -> Vec<LaneProgress> {
        let lanes = self.all();
        let reg = self.registry();
        lanes
            .iter()
            .map(|lane| {
                let label = lane
                    .roots
                    .first()
                    .and_then(|root| reg.probes.get(&PATH_RULES.display(root).to_string()))
                    .and_then(|probe| probe.answer.as_ref())
                    .and_then(|(place, _)| match place {
                        Place::Mounted { drive, .. } => Some(drive.label.clone()),
                        Place::Unplugged | Place::Unknown => None,
                    })
                    .unwrap_or_default();
                LaneProgress { label, progress: lane.inner.ctl.snapshot() }
            })
            .collect()
    }

    /// The controls of the crop thread and the lanes, the crop's first: each one's progress is a line of its own.
    pub(crate) fn controls(&self) -> Vec<Arc<Control>> {
        let mut controls = vec![self.crop_ctl.clone()];
        controls.extend(self.all().into_iter().map(|lane| lane.inner.ctl.clone()));
        controls
    }

    fn wake_coordinator(&self) {
        if let Some(service) = self.coordinator.get() {
            service.wake();
        }
    }

    fn wake_cropper(&self) {
        if let Some(service) = self.cropper.get() {
            service.wake();
        }
    }

    /// Folders to walk: each goes to the lane of its root, once the coordinator has routed it.
    pub(crate) fn request_scans(&self, roots: Vec<RulePath>) {
        {
            let mut reg = self.registry();
            for root in roots {
                if !reg.scans.contains(&root) {
                    reg.scans.push_back(root);
                }
            }
        }
        self.wake_coordinator();
    }

    /// A library folder was removed: the scans asked for under `root` are dropped, and a walk of a folder under it stops.
    /// That walk writes nothing more: the removal took its lease (`Catalog::remove_root`).
    pub(crate) fn stop_scans_under(&self, root: &RulePath) {
        let rules = PATH_RULES;
        {
            let mut reg = self.registry();
            reg.scans.retain(|path| !rules.is_under(path, root));
            for lane in &reg.lanes {
                lane.inner.scans().retain(|path| !rules.is_under(path, root));
                lane.inner.busy().retain(|path| !rules.is_under(path, root));
                let running = lane.inner.scanning.lock().unwrap_or_else(PoisonError::into_inner).clone();
                if running.is_some_and(|path| rules.is_under(&path, root)) {
                    lane.inner.ctl.request_stop();
                    lane.service.wake();
                }
            }
        }
        self.wake_coordinator();
    }

    pub(crate) fn request_crop(&self, ids: Vec<FileId>, folder: PathBuf) {
        self.registry().crops.push_back((ids, folder));
        self.wake_cropper();
    }

    pub(crate) fn request_cut(&self, ids: Vec<FileId>) {
        self.registry().cuts.push_back(ids);
        self.wake_cropper();
    }

    /// The catalog has new work outside a scan (a file was turned): every lane starts a round, and one that is running
    /// stops its round at its next file to do it. `stop` says whether the running rounds stop.
    pub(crate) fn wake_all(&self, stop: bool) {
        for lane in self.all() {
            if stop {
                lane.inner.ctl.request_stop();
            }
            lane.service.wake();
        }
        self.wake_coordinator();
    }

    /// The number of cuts queued and not yet begun (tests: a cut is queued only once confirmed).
    #[cfg(test)]
    pub(crate) fn cuts_queued(&self) -> usize {
        self.registry().cuts.len()
    }

    /// Stops every thread: the coordinator and the crop thread first (so no lane starts after the lanes are asked), then
    /// the lanes, each after its round. `false` if one is still running when `timeout` is spent.
    pub(crate) fn stop(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut stopped = true;
        self.root.request_stop();
        for service in [self.coordinator.get(), self.cropper.get()].into_iter().flatten() {
            service.quit();
            self.root.wake();
            stopped &= service.stop_within(deadline.saturating_duration_since(Instant::now()));
        }
        let lanes = self.all();
        for lane in &lanes {
            lane.inner.ctl.request_stop();
            lane.service.quit();
            lane.inner.ctl.wake();
        }
        for lane in &lanes {
            stopped &= lane.service.stop_within(deadline.saturating_duration_since(Instant::now()));
        }
        stopped
    }

    /// The coordinator's round: the roots' places, the lanes they make, the scans routed, and the lanes retired. Returns
    /// when the coordinator must look again, if it must without a wake (a probe's deadline, an answer's expiry).
    fn route(self: &Arc<Self>, catalog: &mut Option<Catalog>) -> Result<Option<Duration>> {
        let conn = match catalog.take() {
            Some(c) => c,
            None => Catalog::open(&self.shared.catalog_path, PATH_RULES)?,
        };
        let rules = conn.rules();
        let roots: Vec<RulePath> = conn.roots()?.into_iter().map(|r| r.path).collect();
        let now = Instant::now();
        let (retiring, next) = {
            let mut reg = self.registry();
            // The probes are keyed by each root as it is displayed (a path is not ordered, so it is not a key itself).
            let keys: Vec<String> = roots.iter().map(|root| rules.display(root).to_string()).collect();
            reg.probes.retain(|key, _| keys.contains(key));
            for (root, key) in roots.iter().zip(&keys) {
                let probe = reg.probes.entry(key.clone()).or_default();
                if probe.wanted(now) {
                    probe.running = Some(now);
                    if self.spawn_probe(root.clone(), key.clone(), rules).is_err() {
                        // No thread to ask with: the root is unknown until a probe can run, which fails closed (one lane).
                        let probe = reg.probes.entry(key.clone()).or_default();
                        probe.running = None;
                        probe.answer = Some((Place::Unknown, now));
                    }
                }
            }
            let places: Vec<(RulePath, Option<Place>)> = roots
                .iter()
                .zip(&keys)
                .map(|(root, key)| (root.clone(), reg.probes.get(key).and_then(|probe| probe.place(now))))
                .collect();
            let volumes: Vec<(RulePath, i64)> = places
                .iter()
                .filter_map(|(root, place)| match place {
                    Some(Place::Mounted { volume, .. }) => Some((root.clone(), *volume)),
                    _ => None,
                })
                .collect();
            let serial = self.shared.features.get().serial_drives;
            let groups = lanes::plan(&places, serial);
            let retiring = self.assign_lanes(&mut reg, &groups, &volumes)?;
            self.route_scans(&mut reg, &rules, &roots);
            let next = reg.probes.values().filter_map(|probe| probe.due(now)).min();
            (retiring, next.map(|due| due.saturating_duration_since(Instant::now())))
        };
        self.retire(retiring);
        *catalog = Some(conn);
        Ok(next)
    }

    /// Matches the lanes to the groups of roots the plan made: a group keeps the lane that holds most of its roots, and
    /// gets a new lane when none does. The lanes no group keeps are returned to be retired; a kept lane whose volumes changed
    /// is asked to stop, so its round starts again with them.
    fn assign_lanes(
        &self,
        reg: &mut Registry,
        groups: &[Vec<RulePath>],
        volumes: &[(RulePath, i64)],
    ) -> Result<Vec<Lane>> {
        let mut old: Vec<Option<Lane>> = std::mem::take(&mut reg.lanes).into_iter().map(Some).collect();
        let mut kept: Vec<Lane> = Vec::with_capacity(groups.len());
        let mut failure = None;
        for group in groups {
            let best = old
                .iter()
                .enumerate()
                .filter_map(|(index, lane)| {
                    let lane = lane.as_ref()?;
                    let shared = group.iter().filter(|root| lane.roots.contains(root)).count();
                    (shared > 0).then_some((index, shared))
                })
                .max_by_key(|&(index, shared)| (shared, Reverse(index)));
            let lane = match best.and_then(|(index, _)| old.get_mut(index).and_then(Option::take)) {
                Some(lane) => lane,
                None => match self.spawn_lane(reg) {
                    Ok(lane) => lane,
                    Err(e) => {
                        failure = Some(e);
                        break;
                    }
                },
            };
            kept.push(Lane { roots: group.clone(), ..lane });
        }
        // Whatever the spawn failure left unplanned keeps running, as before: the lanes are returned to the registry.
        let mut retiring: Vec<Lane> = old.into_iter().flatten().collect();
        if let Some(e) = failure {
            reg.lanes.append(&mut retiring);
            reg.lanes.append(&mut kept);
            return Err(e);
        }
        let single = kept.len() == 1;
        for lane in &kept {
            let set = if single {
                VolumeSet::All
            } else {
                VolumeSet::Only(lane.roots.iter().filter_map(|root| volume_of(volumes, root)).collect())
            };
            if lane.inner.volume_set() != set {
                lane.inner.ctl.request_stop();
                lane.service.wake();
            }
            lane.inner.set_volumes(set);
        }
        reg.lanes = kept;
        Ok(retiring)
    }

    /// Gives each queued folder to the lane of its root. A folder under no library root is dropped (it isn't in the library);
    /// one whose root has no lane yet waits for it.
    fn route_scans(&self, reg: &mut Registry, rules: &PathRules, roots: &[RulePath]) {
        let pending = std::mem::take(&mut reg.scans);
        for path in pending {
            let Some(root) = roots.iter().find(|root| rules.is_under(&path, root)) else { continue };
            match reg.lanes.iter().find(|lane| lane.roots.contains(root)) {
                Some(lane) => {
                    if !lane.inner.scans().contains(&path) {
                        lane.inner.scans().push_back(path);
                    }
                    // A new folder for a lane: its round stops at the next file, and the folder is walked next.
                    lane.inner.ctl.request_stop();
                    lane.service.wake();
                }
                None => reg.scans.push_back(path),
            }
        }
    }

    /// Stops and ends the retired lanes. Their queued and refused scans go back to the coordinator, to be routed to the
    /// lanes that hold their roots now.
    fn retire(&self, retiring: Vec<Lane>) {
        if retiring.is_empty() {
            return;
        }
        {
            let mut reg = self.registry();
            for lane in &retiring {
                reg.lanes.retain(|kept| kept.inner.number != lane.inner.number);
            }
        }
        for lane in &retiring {
            lane.inner.ctl.request_stop();
            lane.service.quit();
            lane.inner.ctl.wake();
        }
        let deadline = Instant::now() + RETIRE_WAIT;
        for lane in retiring {
            // A lane that doesn't end in time is left to its own end: its walk stops by its lease (`Busy`) if another
            // lane takes its folder meanwhile.
            lane.service.stop_within(deadline.saturating_duration_since(Instant::now()));
            let moved = lane.inner.take_scans();
            self.shared.tally.lane_retired(lane.inner.number);
            let mut reg = self.registry();
            for path in moved {
                if !reg.scans.contains(&path) {
                    reg.scans.push_back(path);
                }
            }
        }
        // The retired lanes' buffer shares go to the lanes left (a lane that ended drops its budget).
        self.governor.rebalance();
        self.wake_coordinator();
    }

    /// A new lane: its state (a child of the root's control), and its thread, which waits for its first round.
    fn spawn_lane(&self, reg: &mut Registry) -> Result<Lane> {
        let number = reg.next_number;
        reg.next_number += 1;
        let inner = Arc::new(Inner::new(self.shared.clone(), number, self.root.lane(), self.governor.lane()));
        let job = inner.clone();
        let service =
            Service::start("background job", move |waker| job.run(waker)).context("start a background job lane")?;
        Ok(Lane { inner, service: Arc::new(service), roots: Vec::new() })
    }

    /// Asks a root's drive on a thread of its own. Its answer is kept on the root's probe, and wakes the coordinator.
    fn spawn_probe(self: &Arc<Self>, root: RulePath, key: String, rules: PathRules) -> std::io::Result<()> {
        let weak = Arc::downgrade(self);
        let drives = self.drives.clone();
        std::thread::Builder::new().name("drive probe".into()).spawn(move || {
            let place = probe_place(rules, &root, drives.as_ref());
            if let Some(lanes) = weak.upgrade() {
                {
                    let mut reg = lanes.registry();
                    if let Some(probe) = reg.probes.get_mut(&key) {
                        probe.answer = Some((place, Instant::now()));
                        probe.running = None;
                    }
                }
                lanes.wake_coordinator();
            }
        })?;
        Ok(())
    }

    /// Holds the lanes that own `volumes`, and waits until each has ended its round: a crop or a cut is about to change
    /// files on those volumes, which the lanes read. `None` when the crop is stopped first (the holds are given back).
    fn park(&self, volumes: &BTreeSet<i64>) -> Option<Vec<Hold>> {
        let owners: Vec<Lane> = self
            .all()
            .into_iter()
            .filter(|lane| {
                let set = lane.inner.volume_set();
                volumes.iter().any(|&volume| set.contains(volume))
            })
            .collect();
        let holds: Vec<Hold> = owners
            .iter()
            .map(|lane| {
                let hold = lane.inner.ctl.hold();
                lane.inner.ctl.request_stop();
                lane.service.wake();
                hold
            })
            .collect();
        for lane in &owners {
            if !lane.inner.wait_idle(&self.crop_ctl) {
                return None;
            }
        }
        Some(holds)
    }

    /// One crop or cut: its files' volumes are parked, and it runs on a connection of the crop thread's.
    fn job(&self, conn: &mut Option<Catalog>, job: Job) -> Result<Done> {
        let catalog = match conn.take() {
            Some(c) => c,
            None => Catalog::open(&self.shared.catalog_path, PATH_RULES)?,
        };
        let ids = match &job {
            Job::Save(ids, _) | Job::Cut(ids) => ids,
        };
        let volumes = catalog.volumes_of(ids)?;
        let Some(_holds) = self.park(&volumes) else {
            *conn = Some(catalog);
            return Ok(Done::Stopped(job));
        };
        let opts = Options::new(Priority::BelowNormal);
        let done = match job {
            Job::Save(ids, folder) => match pg_crop::save(&catalog, &ids, &folder, &opts, &self.crop_ctl) {
                Ok(s) if !s.remaining.is_empty() => Done::Stopped(Job::Save(s.remaining, folder)),
                Ok(s) => {
                    self.shared.notices.post(Notice::Cropped {
                        folder: folder.display().to_string(),
                        saved: count(s.saved),
                        skipped: count(s.skipped),
                        failed: count(s.failed),
                        failures: s.failures.into_iter().map(file_failure).collect(),
                    });
                    Done::Finished
                }
                Err(e) => {
                    self.shared.notices.post(Notice::CropFailed { error: error_text(&e) });
                    Done::Finished
                }
            },
            Job::Cut(ids) => match pg_crop::cut(&catalog, &ids, true, &opts, &self.crop_ctl) {
                Ok(s) => {
                    // The cut's files are forgotten and analyzed again, whether or not the cut is done.
                    cut_done(&self.shared, &s.cut);
                    if s.remaining.is_empty() {
                        self.shared.notices.post(Notice::BordersCut {
                            saved: count(s.saved),
                            skipped: count(s.skipped),
                            unsupported: count(s.unsupported),
                            failed: count(s.failed),
                            failures: s.failures.into_iter().map(file_failure).collect(),
                        });
                        Done::Finished
                    } else {
                        Done::Stopped(Job::Cut(s.remaining))
                    }
                }
                Err(e) => {
                    self.shared.notices.post(Notice::CutFailed { error: error_text(&e) });
                    Done::Finished
                }
            },
        };
        *conn = Some(catalog);
        Ok(done)
    }

    /// Runs the queued crops, then the cuts, until none is left or the thread quits. A stopped one keeps its rest at the
    /// front of its queue.
    fn run_jobs(&self, conn: &mut Option<Catalog>, waker: &Waker<'_>) {
        loop {
            if waker.quitting() {
                return;
            }
            let next = {
                let mut reg = self.registry();
                match reg.crops.pop_front() {
                    Some((ids, folder)) => Job::Save(ids, folder),
                    None => match reg.cuts.pop_front() {
                        Some(ids) => Job::Cut(ids),
                        None => return,
                    },
                }
            };
            let is_cut = matches!(next, Job::Cut(_));
            match panics::catch(AssertUnwindSafe(|| self.job(conn, next))) {
                Ok(Ok(Done::Finished)) => {}
                Ok(Ok(Done::Stopped(rest))) => {
                    let mut reg = self.registry();
                    match rest {
                        Job::Save(ids, folder) => reg.crops.push_front((ids, folder)),
                        Job::Cut(ids) => reg.cuts.push_front(ids),
                    }
                    return;
                }
                Ok(Err(e)) => {
                    let error = error_text(&e);
                    if is_cut {
                        self.shared.notices.post(Notice::CutFailed { error });
                    } else {
                        self.shared.notices.post(Notice::CropFailed { error });
                    }
                }
                Err(panic) => {
                    *conn = None;
                    let error = tidy(panics::message(&*panic), ERROR_CAP);
                    if is_cut {
                        self.shared.notices.post(Notice::CutFailed { error });
                    } else {
                        self.shared.notices.post(Notice::CropFailed { error });
                    }
                }
            }
        }
    }
}

/// The coordinator's thread: a round each time it is woken, or when the next probe is due.
fn coordinate(weak: &Weak<Lanes>, waker: &Waker<'_>) {
    let mut catalog: Option<Catalog> = None;
    let mut next: Option<Duration> = None;
    loop {
        if waker.wait(next) == Woke::Quit {
            return;
        }
        let Some(lanes) = weak.upgrade() else { return };
        next = match lanes.route(&mut catalog) {
            Ok(next) => {
                lanes.shared.tally.round(COORDINATOR, None);
                next
            }
            Err(e) => {
                lanes.shared.tally.round(COORDINATOR, Some(error_text(&e)));
                Some(PLACE_TTL)
            }
        };
        drop(lanes);
    }
}

/// The crop thread: the queued crops and cuts, each time it is woken.
fn crop(weak: &Weak<Lanes>, waker: &Waker<'_>) {
    let mut catalog: Option<Catalog> = None;
    loop {
        if waker.wait(None) == Woke::Quit {
            return;
        }
        let Some(lanes) = weak.upgrade() else { return };
        lanes.run_jobs(&mut catalog, waker);
        drop(lanes);
    }
}

/// Where `root` is, as the lanes need it: its volume (as the catalog stores it) and its drive, or that it is not there, or
/// that it can't be told.
pub(crate) fn probe_place(rules: PathRules, root: &RulePath, drives: &dyn Drives) -> Place {
    let folder = pg_platform::names::disk_path(rules, root);
    let volume = match pg_scan::root_volume(rules, root) {
        Ok(volume) => volume,
        Err(e) => return if unplugged(&e) { Place::Unplugged } else { Place::Unknown },
    };
    let drive = match drives.of(&folder) {
        Ok(drive) => drive,
        Err(e) => return if unplugged(&e) { Place::Unplugged } else { Place::Unknown },
    };
    match i64::try_from(volume) {
        Ok(volume) => Place::Mounted { volume, drive },
        Err(_) => Place::Unknown,
    }
}
