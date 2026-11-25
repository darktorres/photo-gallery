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

use crate::{Error, Result};
use pg_catalog::{Catalog, VolumeSet};
use pg_jobs::control::{Control, Hold};
use pg_jobs::governor::Governor;
use pg_jobs::panics;
use pg_jobs::pipeline::Options;
use pg_jobs::service::{Service, Waker, Woke};
use pg_jobs::tuner::MAX_READERS;
use pg_model::ids::FileId;
use pg_model::job::Change;
use pg_model::notice::{ERROR_CAP, Notice, tidy};
use pg_model::progress::LaneProgress;
use pg_model::progress::{Phase, Progress};
use pg_model::removal::Volumes;
use pg_model::rules::{Path as RulePath, PathRules};
use pg_platform::drive::Drives;
use pg_platform::native::PATH_RULES;
use pg_platform::priority::Priority;

use crate::inner::Inner;
use crate::lanes::{self, Place, probe_place};
use crate::passes::{count, cut_done, error_text, file_failure};
use crate::removals;
use crate::shared::Shared;

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
/// The free memory the lanes leave to everything else: unmeasured, provisional (`docs/plans/ADAPTIVE_READERS_PLAN.md`).
const MEMORY_HEADROOM: u64 = 1 << 30;
/// How often the coordinator looks at the machine's free memory, and moves the lanes' cap when it moved.
const GOVERN_EVERY: Duration = Duration::from_secs(5);

/// The lanes' soft cap now, with `held` bytes in their pools (`pg_jobs::governor::soft_cap`).
fn current_cap(held: u64) -> Option<u64> {
    pg_jobs::governor::soft_cap(
        pg_platform::memory::physical_bytes(),
        pg_platform::memory::available_bytes(),
        held,
        MEMORY_SHARE,
        MEMORY_HEADROOM,
    )
}

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
    /// The files turned since the coordinator last routed: the lanes of their volumes start again (`Lanes::wake_files`).
    turned: Vec<FileId>,
}

/// The indexes of the lanes whose volumes include one of `volumes`: the lanes that read the files on those volumes.
pub(crate) fn lanes_with(sets: &[VolumeSet], volumes: &BTreeSet<u64>) -> Vec<usize> {
    sets.iter()
        .enumerate()
        .filter(|(_, set)| volumes.iter().any(|&volume| set.contains(volume)))
        .map(|(index, _)| index)
        .collect()
}

/// The volume of `root` among `volumes`, if it has one.
fn volume_of(volumes: &[(RulePath, u64)], root: &RulePath) -> Option<u64> {
    volumes.iter().find(|(path, _)| path == root).map(|(_, volume)| *volume)
}

/// The job's lanes, and the coordinator and crop threads beside them.
pub(crate) struct Lanes {
    /// The run's root control: its global pause, and its stop reaches every lane and the crop.
    pub(crate) root: Arc<Control>,
    /// The crop thread's control: a child of the root, shown as a job of its own.
    crop_ctl: Arc<Control>,
    /// The removals' control: a child of the root too, shown as a job of its own (its progress, and its pause).
    pub(crate) removal_ctl: Arc<Control>,
    pub(crate) shared: Arc<Shared>,
    /// Splits the file-buffer budget among the lanes (each lane's [`Inner::options`] carries its share).
    governor: Governor,
    /// Where the roots' drives are asked (the system's, or a test's).
    drives: Arc<dyn Drives + Send + Sync>,
    registry: Mutex<Registry>,
    coordinator: OnceLock<Arc<Service>>,
    cropper: OnceLock<Arc<Service>>,
    removals: OnceLock<Arc<Service>>,
}

/// How often the removals are looked at without a wake: a removal kept for an unfinished write is tried again then.
const REMOVAL_RECHECK: Duration = Duration::from_secs(30);

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
        let removal_ctl = root.lane();
        // The lanes' CPU is shared: their workers take permits, so two drives' decoders share the cores.
        let cpus = pg_jobs::pipeline::cpus();
        root.share_cpu(cpus);
        // The lanes together hold at most a quarter of the machine's memory for file buffers, and no more than is free
        // beyond a headroom (`soft_cap`, which the coordinator keeps up to date), each up to the knee: the budget of the
        // most readers a tuned pass takes. The budget stays at that static most: a lane that reads with fewer holds less
        // through fewer reads in flight, and no lane's decisions move another's share.
        let knee = Options::budget(MAX_READERS, pg_jobs::pipeline::default_workers())
            .map_or(Options::new(Priority::BelowNormal).buffer_bytes.get(), |bytes| bytes.get());
        let governor = Governor::new(knee, current_cap(0));
        let lanes = Arc::new(Lanes {
            root,
            crop_ctl,
            removal_ctl,
            shared,
            governor,
            drives,
            registry: Mutex::new(Registry::default()),
            coordinator: OnceLock::new(),
            cropper: OnceLock::new(),
            removals: OnceLock::new(),
        });
        // The removals an earlier run left are parked before any lane starts (D5), and released by the removal job.
        park_earlier_removals(&lanes.shared);
        let weak = Arc::downgrade(&lanes);
        let coordinator = Service::start("lane coordinator", move |waker| coordinate(&weak, waker))
            .map_err(|e| Error::step("can't start the lane coordinator", e))?;
        let _ = lanes.coordinator.set(Arc::new(coordinator));
        let weak = Arc::downgrade(&lanes);
        let cropper = Service::start("crop job", move |waker| crop(&weak, waker))
            .map_err(|e| Error::step("start the crop job", e))?;
        let _ = lanes.cropper.set(Arc::new(cropper));
        let weak = Arc::downgrade(&lanes);
        let removals = Service::start("removal job", move |waker| remove(&weak, waker))
            .map_err(|e| Error::step("start the removal job", e))?;
        let _ = lanes.removals.set(Arc::new(removals));
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
        self.all()
            .iter()
            .map(|lane| {
                // One snapshot for both, so a row's phase and its time since work agree.
                let (progress, since_work) = lane.inner.ctl.snapshot_since();
                LaneProgress {
                    number: lane.inner.number,
                    label: lane.inner.label(),
                    progress,
                    since_work,
                    paused: lane.inner.ctl.is_own_paused(),
                    readers: lane.inner.readers_now(),
                }
            })
            .collect()
    }

    /// Pauses or resumes the lane numbered `number` alone. `false` when no lane has that number (it has retired).
    pub(crate) fn set_lane_paused(&self, number: usize, paused: bool) -> bool {
        match self.all().iter().find(|lane| lane.inner.number == number) {
            Some(lane) => {
                lane.inner.ctl.set_paused(paused);
                true
            }
            None => false,
        }
    }

    /// The crop thread's progress and time since work, read together.
    pub(crate) fn crop_snapshot(&self) -> (Progress, Option<Duration>) {
        self.crop_ctl.snapshot_since()
    }

    /// The controls of the crop thread and the lanes, the crop's first: each one's progress is a line of its own.
    pub(crate) fn controls(&self) -> Vec<Arc<Control>> {
        let mut controls = vec![self.crop_ctl.clone(), self.removal_ctl.clone()];
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

    /// A removal was begun (`Catalog::begin_removal`): the removals thread finishes it, now.
    pub(crate) fn wake_removals(&self) {
        if let Some(service) = self.removals.get() {
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

    /// Whether every request has been given to a lane: no crop or cut is waiting for one, no turned file is waiting for its lane to
    /// start again, and, when `scans` (the scan pass runs), no scan is waiting either (`Runner::quiescent`). A scan waiting for a
    /// root with no lane is waiting too.
    pub(crate) fn unrouted(&self, scans: bool) -> bool {
        let reg = self.registry();
        (!scans || reg.scans.is_empty()) && reg.crops.is_empty() && reg.cuts.is_empty() && reg.turned.is_empty()
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

    /// Files were turned (their analysis is stale): the lanes of their volumes stop their rounds and start again, and the
    /// other lanes keep theirs. A file the catalog doesn't know has no volume, so every lane is woken for it.
    pub(crate) fn wake_files(&self, ids: Vec<FileId>) {
        if ids.is_empty() {
            return;
        }
        self.registry().turned.extend(ids);
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
        for service in [self.coordinator.get(), self.cropper.get(), self.removals.get()].into_iter().flatten() {
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
        let turned = std::mem::take(&mut self.registry().turned);
        if !turned.is_empty() {
            // The volumes the turned files are on. A file with no volume (not in the catalog, or the lookup failed) wakes every
            // lane: the safe answer, since no lane can be shown to be unaffected.
            let volumes: BTreeSet<u64> = conn.volumes_of(&turned).unwrap_or_default();
            let reg = self.registry();
            let sets: Vec<VolumeSet> = reg.lanes.iter().map(|lane| lane.inner.volume_set()).collect();
            let wake = if volumes.is_empty() { (0..sets.len()).collect() } else { lanes_with(&sets, &volumes) };
            for (index, lane) in reg.lanes.iter().enumerate() {
                if wake.contains(&index) {
                    lane.inner.ctl.request_stop();
                    lane.service.wake();
                }
            }
        }
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
            let volumes: Vec<(RulePath, u64)> = places
                .iter()
                .filter_map(|(root, place)| match place {
                    Some(Place::Mounted { volume, .. }) => Some((root.clone(), *volume)),
                    _ => None,
                })
                .collect();
            let groups = lanes::plan(&places);
            let before = lane_names(&reg.lanes);
            let retiring = self.assign_lanes(&mut reg, &groups, &volumes)?;
            for lane in &reg.lanes {
                let place_of =
                    |root: &RulePath| places.iter().find(|(path, _)| path == root).and_then(|(_, place)| place.clone());
                lane.inner.set_label(crate::lanes::drive_label(&lane.roots, place_of));
                // The lane's tuners follow its drive: bound on every round, started again only when it changed.
                lane.inner.bind_drive(crate::lanes::lane_drive(&lane.roots, place_of).as_ref());
            }
            // A label that changed and a lane that retired change the rows without any progress event: tell the hook.
            // The hook only publishes on the bus, so it is safe under the registry lock.
            if before != lane_names(&reg.lanes) {
                self.root.announce();
            }
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
        volumes: &[(RulePath, u64)],
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
            let set = crate::lanes::lane_volumes(single, lane.roots.iter().filter_map(|root| volume_of(volumes, root)));
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
            // What the lane learned of its drive is kept for the next lane on it.
            lane.inner.save_profiles();
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
        let service = Service::start("background job", move |waker| job.run(waker))
            .map_err(|e| Error::step("start a background job lane", e))?;
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
    pub(crate) fn park(&self, volumes: &BTreeSet<u64>) -> Option<Vec<Hold>> {
        self.park_owners(&Volumes { volumes: volumes.clone(), all: false }, &self.crop_ctl)
    }

    /// The lanes that own `parked`'s volumes (every lane when `parked.all`).
    fn lanes_on(&self, parked: &Volumes) -> Vec<Lane> {
        let all = self.all();
        if parked.all {
            return all;
        }
        let sets: Vec<VolumeSet> = all.iter().map(|lane| lane.inner.volume_set()).collect();
        lanes_with(&sets, &parked.volumes).into_iter().filter_map(|index| all.get(index).cloned()).collect()
    }

    /// Holds the lanes that own `parked`'s volumes, asks them to stop, and waits until each has ended its round. `stop` is
    /// the caller's control: when it is asked first, `None` (the holds are given back, and nothing was changed).
    pub(crate) fn park_owners(&self, parked: &Volumes, stop: &Control) -> Option<Vec<Hold>> {
        let owners = self.lanes_on(parked);
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
            if !lane.inner.wait_idle(stop) {
                return None;
            }
        }
        Some(holds)
    }

    /// A removal began (`Begun::Started`): its lanes are parked now, without waiting, so no lane starts a round on the
    /// folder's volumes from here (section 1, D1). The rounds running are asked to stop; the removal job waits for them and
    /// takes its own holds before it hides the files.
    pub(crate) fn park_removal(&self, path: &RulePath, volumes: &Volumes) {
        self.shared.parks.park(&park_key(path), volumes.clone());
        for lane in self.lanes_on(volumes) {
            lane.inner.ctl.request_stop();
            lane.service.wake();
        }
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
            // The presence check runs on its own connection, shared by the cut's writers: a folder removed while the cut runs
            // is not written to.
            Job::Cut(ids) => match Catalog::open(&self.shared.catalog_path, PATH_RULES)
                .map(Mutex::new)
                .map_err(Error::from)
                .and_then(|checker| {
                    let present = |id: FileId| checker.lock().is_ok_and(|c| c.is_present(id).unwrap_or(false));
                    pg_crop::cut(&catalog, &ids, true, &opts, &self.crop_ctl, &present).map_err(Error::from)
                }) {
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

/// The coordinator's thread: a round each time it is woken, or when the next probe is due; and the lanes' memory cap,
/// looked at every [`GOVERN_EVERY`].
fn coordinate(weak: &Weak<Lanes>, waker: &Waker<'_>) {
    let mut catalog: Option<Catalog> = None;
    let mut next: Option<Duration> = None;
    let mut governed = Instant::now();
    loop {
        let wait = Some(next.map_or(GOVERN_EVERY, |next| next.min(GOVERN_EVERY)));
        if waker.wait(wait) == Woke::Quit {
            return;
        }
        let Some(lanes) = weak.upgrade() else { return };
        if governed.elapsed() >= GOVERN_EVERY {
            governed = Instant::now();
            lanes.govern();
        }
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

impl Lanes {
    /// Moves the lanes' soft cap to the machine's free memory now, and rebalances their shares when it moved by more than
    /// the hysteresis (`Governor::set_cap`).
    fn govern(&self) {
        if self.governor.set_cap(current_cap(self.governor.held())) {
            self.governor.rebalance();
        }
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

/// The removals thread: woken when a folder is removed, and every [`REMOVAL_RECHECK`] otherwise, it finishes the removals
/// not finished (`removals::finish`): a removal kept for an unfinished write is tried again, and one a command left (with
/// the app closed) is finished here.
fn remove(weak: &Weak<Lanes>, waker: &Waker<'_>) {
    let mut catalog: Option<Catalog> = None;
    // The failure last told: a failure that repeats at each recheck is told once, until it changes or goes.
    let mut told: Option<String> = None;
    // The thumbnails of the files a command removed (with the app closed) are dropped once, at the first start.
    let mut swept = false;
    // The hidden folders no marker covers (an older build's removals) are given one, once the thread has run without error.
    let mut adopted = false;
    // The first pass is at once, not after a recheck: a removal a command left (with the app closed) is finished at start.
    let mut first = true;
    loop {
        if std::mem::take(&mut first) {
            if waker.quitting() {
                return;
            }
        } else if waker.wait(Some(REMOVAL_RECHECK)) == Woke::Quit {
            return;
        }
        let Some(lanes) = weak.upgrade() else { return };
        if !swept {
            swept = true;
            if let Err(e) = lanes.shared.thumbs.sweep_orphans(&lanes.shared.catalog_path) {
                lanes.shared.notices.post(Notice::FolderNotRemoved {
                    error: format!("the thumbnails of removed files weren't dropped: {}", error_text(&e)),
                });
            }
        }
        match lanes.run_removals(&mut catalog, !adopted) {
            Ok(true) => {
                told = None;
                adopted = true;
            }
            // A stop: the markers wait, parked, for the next try.
            Ok(false) => {}
            Err(error) => {
                if told.as_deref() != Some(error.as_str()) {
                    lanes.shared.notices.post(Notice::FolderNotRemoved { error: error.clone() });
                    told = Some(error);
                }
            }
        }
        drop(lanes);
    }
}

impl Lanes {
    /// Finishes the removals not finished, as one phase of the removal job: its row in the status bar, its progress
    /// (the files of the removals, as they go), and its pause, which waits between batches. The error text when a
    /// removal could not be finished.
    fn run_removals(&self, conn: &mut Option<Catalog>, adopt: bool) -> std::result::Result<bool, String> {
        let catalog = match conn.take() {
            Some(c) => c,
            None => Catalog::open(&self.shared.catalog_path, PATH_RULES)
                .map_err(|e| error_text(&pg_model::error::chain(&e)))?,
        };
        let found = if adopt { catalog.adopt_hidden_folders().map(drop) } else { Ok(()) }
            .and_then(|()| catalog.removal_markers());
        let result = match found {
            Ok(markers) if markers.is_empty() => Ok(true),
            Ok(markers) => self.finish_removals(&catalog, &markers),
            Err(e) => Err(error_text(&pg_model::error::chain(&e))),
        };
        // The removals an earlier run left are no longer unknown once the job has been through all of them.
        if matches!(result, Ok(true)) {
            self.shared.parks.release(STARTUP_KEY);
        }
        *conn = Some(catalog);
        result
    }

    /// The markers' removals, finished (section 1). Each marker is hidden under a park of its lanes (`hide_marker`). `false`
    /// when a stop came first: the markers wait for the next try, parked. Then the files go in batches under the job's
    /// phase, and the folders' listing is told the tree changed. No lane is parked for the deletes: the hidden files are
    /// invisible to every pass.
    fn finish_removals(&self, catalog: &Catalog, markers: &[pg_catalog::Marker]) -> std::result::Result<bool, String> {
        for marker in markers {
            self.stop_scans_under(&marker.path);
        }
        for marker in markers {
            if !self.hide_marker(catalog, &marker.path)? {
                return Ok(false);
            }
        }
        let total = catalog.removal_files().unwrap_or(0);
        let _phase = self.removal_ctl.begin(Phase::Removing, total);
        let mut done = 0u64;
        let finished = removals::drive(catalog, &self.shared.thumbs, &self.shared.catalog_path, &mut |files| {
            done = done.saturating_add(u64::try_from(files).unwrap_or(u64::MAX));
            self.removal_ctl.set_done(done);
            self.removal_ctl.checkpoint()
        });
        self.shared.changes.publish(Change::Scanned);
        finished.map(|_| true).map_err(|e| error_text(&e))
    }

    /// One marker's hide (section 1, D1): its lanes are parked on the folder's volumes, its files are hidden in batches,
    /// the grid is told (D4), and then the park is released. A stop, or a failed hide, keeps the park: the folder may be
    /// partly hidden, and no lane may work on it until the hide completes at a later try. `false` when a stop came while
    /// the lanes were being parked.
    fn hide_marker(&self, catalog: &Catalog, path: &RulePath) -> std::result::Result<bool, String> {
        #[cfg(test)]
        self.shared.hooks.at_hide(path);
        let key = park_key(path);
        let volumes = catalog.removal_volumes(path).map_err(|e| error_text(&pg_model::error::chain(&e)))?;
        self.shared.parks.park(&key, volumes.clone());
        let Some(holds) = self.park_owners(&volumes, &self.removal_ctl) else {
            return Ok(false);
        };
        // A test's failure here is the failure of the hide with the park held (see `TestHooks::on_parked`). The holds drop
        // on the way out, and the park stays: the folder may be partly hidden.
        #[cfg(test)]
        self.shared.hooks.at_parked(path).map_err(|e| format!("the hide failed (a test's failure): {e}"))?;
        let hidden = catalog.hide_pages(path);
        self.shared.changes.publish(Change::Scanned);
        drop(holds);
        hidden.map_err(|e| error_text(&pg_model::error::chain(&e)))?;
        self.shared.parks.release(&key);
        Ok(true)
    }
}

/// The key a removal parks under: its folder as the rules display it.
fn park_key(path: &RulePath) -> String {
    PATH_RULES.display(path).to_string()
}

/// The key of the parks the removals an earlier run left have, when their markers can't be read: released when those
/// removals run to their end.
const STARTUP_KEY: &str = "startup removals";

/// Parks the volumes of the removals an earlier run left, before any lane starts (section 1, D5): no lane works on a folder
/// whose files are not hidden yet. A marker whose volumes can't be read parks every lane, under [`STARTUP_KEY`], until the
/// removals run. No catalog yet means no removal.
fn park_earlier_removals(shared: &Shared) {
    let Ok(catalog) = Catalog::open(&shared.catalog_path, PATH_RULES) else { return };
    match catalog.removal_markers() {
        Ok(markers) => {
            for marker in markers {
                let volumes =
                    catalog.removal_volumes(&marker.path).unwrap_or(Volumes { volumes: BTreeSet::new(), all: true });
                shared.parks.park(&park_key(&marker.path), volumes);
            }
        }
        Err(_) => shared.parks.park(STARTUP_KEY, Volumes { volumes: BTreeSet::new(), all: true }),
    }
}

/// Each lane's number and label, the two things the rows show that no progress event carries.
fn lane_names(lanes: &[Lane]) -> Vec<(usize, String)> {
    lanes.iter().map(|lane| (lane.inner.number, lane.inner.label())).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn volumes(of: &[u64]) -> BTreeSet<u64> {
        of.iter().copied().collect()
    }

    /// The lanes that read the files on a volume are the ones whose volumes include it: a turned file or a crop wakes or
    /// parks only those (L5), and every other lane keeps its round.
    #[test]
    fn a_volume_belongs_to_the_lanes_that_read_it() {
        let sets = [VolumeSet::Only(vec![1, 3]), VolumeSet::Only(vec![7]), VolumeSet::All];
        assert_eq!(
            lanes_with(&sets, &volumes(&[7])),
            vec![1, 2],
            "the lane of 7, and the lane that reads every volume"
        );
        assert_eq!(lanes_with(&sets, &volumes(&[3])), vec![0, 2]);
        assert_eq!(lanes_with(&sets, &volumes(&[9])), vec![2], "only the lane that reads every volume reads 9");
    }

    #[test]
    fn no_volume_is_no_lane() {
        let sets = [VolumeSet::Only(vec![1]), VolumeSet::Only(vec![2])];
        assert!(lanes_with(&sets, &volumes(&[])).is_empty(), "the caller decides what an unknown volume wakes");
        assert!(lanes_with(&[], &volumes(&[1])).is_empty());
    }
}
