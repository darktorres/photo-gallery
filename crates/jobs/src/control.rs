//! What a running pass shares with whoever started it: its progress, pause, and a stop request (a new job, a new
//! thumbnail height, quitting). It holds no text: whoever shows the progress formats it (the command line, the app).
//! The phase and the progress are the model's (`pg_model::progress`); the clock they are timed by is here.
//!
//! A run can have lanes (one per drive, `docs/plans/MULTI_DRIVE_LANES_PLAN.md`): [`Control::lane`] makes a child of the
//! root that has its own stop, its own pause and its own progress, and shares the root's global pause, its stop, the
//! hooks and a bus that tags every lane's events with its [`LaneId`] ([`Control::subscribe_all`]). The root's pause
//! stops every lane's clock; a lane's pause stops that lane alone; the root's stop stops every lane.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock, PoisonError, Weak};
use std::time::{Duration, Instant};

use pg_model::num::count_u64;
use pg_model::progress::{Mark, Phase, Progress};

use crate::events::{Bus, Subscription, Watch};
use crate::governor::{CpuGate, Permit};

/// The rate and the time left are taken over this much of the phase's latest progress: a sliding window.
const RATE_WINDOW: Duration = Duration::from_secs(30);
/// Marks of the count go down at most this often, so a window holds about `RATE_WINDOW / MARK_EVERY` of them.
const MARK_EVERY: Duration = Duration::from_millis(500);

/// The progress of the phase running, when it began, and the marks of its count the rate window is taken from.
#[derive(Clone)]
struct Running {
    progress: Progress,
    started: Instant,
    /// The pauses of the phase: the time spent paused, and the pause in progress.
    pauses: PauseClock,
    /// Files that finished while paused. They took no working time, so the rate leaves them out.
    drained: u64,
    /// (working time, count without the drained files), oldest first. The front one is where the window starts: the
    /// latest mark at least a window old, or the phase's start.
    marks: VecDeque<(Duration, u64)>,
    /// When the last phase ended: `None` until the control's first phase has ended.
    ended: Option<Instant>,
}

impl Running {
    fn begun(progress: Progress, started: Instant) -> Running {
        Running {
            progress,
            started,
            pauses: PauseClock::default(),
            drained: 0,
            marks: VecDeque::from([(Duration::ZERO, 0)]),
            ended: None,
        }
    }

    fn idle() -> Running {
        Running::begun(Progress::idle(), Instant::now())
    }

    /// The idle state a phase leaves behind, remembering when it ended.
    fn ended(now: Instant) -> Running {
        Running { ended: Some(now), ..Running::idle() }
    }

    /// The time since work last ran: `Some(0)` while a phase runs, the time since the last one ended, `None` before
    /// any phase has ended.
    fn since_work(&self, now: Instant) -> Option<Duration> {
        if self.progress.phase != Phase::Idle {
            Some(Duration::ZERO)
        } else {
            self.ended.map(|at| now.saturating_duration_since(at))
        }
    }

    /// The time the phase has run at `now`, not counting its pauses.
    fn active(&self, now: Instant) -> Duration {
        now.saturating_duration_since(self.started).saturating_sub(self.pauses.total_at(now))
    }

    /// Pauses or resumes the phase's clock at `now`. A repeat changes nothing.
    fn set_paused(&mut self, paused: bool, now: Instant) {
        self.pauses.set(paused, now);
    }

    /// Applies a change of the progress at `now`. A file finished while paused is drained, not counted in the window.
    fn change(&mut self, now: Instant, change: impl FnOnce(&mut Progress)) {
        let before = self.progress.done;
        change(&mut self.progress);
        if self.pauses.since.is_some() {
            self.drained += self.progress.done.saturating_sub(before);
        }
        self.mark(now);
    }

    /// Notes the count at `now` (when it is due), and forgets the marks the window no longer starts from.
    fn mark(&mut self, now: Instant) {
        let active = self.active(now);
        let counted = self.progress.done.saturating_sub(self.drained);
        let due = self.marks.back().is_none_or(|&(at, _)| active.saturating_sub(at) >= MARK_EVERY);
        if due {
            self.marks.push_back((active, counted));
        }
        while self.marks.get(1).is_some_and(|&(at, _)| at + RATE_WINDOW <= active) {
            self.marks.pop_front();
        }
    }

    /// The progress as it is at `now`, with the working time the phase has run and the window's start. `since.done` is
    /// set so that `done - since.done` is the files finished in the window, drained ones left out.
    fn read(&self, now: Instant) -> Progress {
        let (at, counted) = self.marks.front().copied().unwrap_or((Duration::ZERO, 0));
        Progress {
            elapsed: self.active(now),
            since: Mark { at, done: counted.saturating_add(self.drained) },
            ..self.progress.clone()
        }
    }
}

/// A progress report, or the end of a phase (with the progress it ended with).
#[derive(Clone, Debug)]
pub enum Event {
    /// A progress report while the phase runs.
    Progress(Progress),
    /// The end of the phase, with the progress it ended with.
    Finished(Progress),
}

/// A lane of a run: the root is [`LaneId::ROOT`], and each [`Control::lane`] takes the next number.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LaneId(u32);

impl LaneId {
    /// The root control's lane: the run's own pass, when it has no lanes.
    pub const ROOT: LaneId = LaneId(0);

    /// The lane's number (0 for the root).
    pub fn get(self) -> u32 {
        self.0
    }
}

/// An event of one lane ([`Control::subscribe_all`]).
#[derive(Clone, Debug)]
pub struct Tagged {
    /// The lane the event is from.
    pub lane: LaneId,
    /// The event itself.
    pub event: Event,
}

/// Progress events come at most this often while a phase runs.
const EVENT_INTERVAL: Duration = Duration::from_secs(5);
/// A change of the count reaches the hook at most this often, across the run's lanes (about 10 a second): a pass with
/// fast files would otherwise call it for every file.
const NOTICE_INTERVAL: Duration = Duration::from_millis(100);
/// Events a subscriber can fall behind by before the oldest are dropped.
const EVENT_CAP: usize = 64;

/// What a run's controls share: the global pause, the root's stop, the hooks, the lanes, and the tagged bus.
struct Family {
    /// The root's pause: every lane waits, and every lane's clock stops.
    global_paused: AtomicBool,
    /// The root's stop: every lane stops.
    root_stop: AtomicBool,
    /// Called whenever what `snapshot` or `is_paused` says changes (progress, the pause), for an owner that reads them on
    /// its own thread instead of polling. A change of the count is throttled to [`NOTICE_INTERVAL`]; the others are not.
    hook: OnceLock<Box<dyn Fn() + Send + Sync>>,
    /// When the hook last ran for a change of the count, across the run's lanes.
    last_notice: Mutex<Option<Instant>>,
    /// Called when a lane changed the failure ledger ([`Control::ledger_changed`]).
    on_ledger: OnceLock<Box<dyn Fn() + Send + Sync>>,
    next_lane: AtomicU32,
    /// The run's CPU permits, shared by every lane's workers ([`Control::share_cpu`]); none until it is set.
    cpu: OnceLock<Arc<CpuGate>>,
    /// The lanes made by [`Control::lane`], held weakly: a lane lasts while its owner holds it.
    lanes: Mutex<Vec<Weak<Control>>>,
    /// Every lane's events, tagged with the lane (the root's included).
    everything: Bus<Tagged>,
}

impl Family {
    fn new() -> Family {
        Family {
            global_paused: AtomicBool::new(false),
            root_stop: AtomicBool::new(false),
            hook: OnceLock::new(),
            last_notice: Mutex::new(None),
            on_ledger: OnceLock::new(),
            next_lane: AtomicU32::new(0),
            cpu: OnceLock::new(),
            lanes: Mutex::new(Vec::new()),
            everything: Bus::new(EVENT_CAP),
        }
    }
}

/// One lane's handle on its job (or the root's, for the job as a whole): its stop and pause, its progress and its
/// events. The lanes of one job share a `Family`, which holds what they have in common.
pub struct Control {
    lane: LaneId,
    family: Arc<Family>,
    /// This lane's own stop (the root never sets it: its stop is the family's).
    stop: AtomicBool,
    /// This lane's own pause and holds, under the lock its waiters wait on (`resumed`). The root's own pause is never set:
    /// its pause is the family's global one.
    paused: Mutex<Pausing>,
    resumed: Condvar,
    progress: Watch<Running>,
    events: Bus<Event>,
    /// When the last progress event went out.
    last_event: Mutex<Instant>,
    /// The pauses since the job started, for the perf log's pass records (a pass's pauses outside its phases count too).
    pauses: Mutex<PauseClock>,
}

/// The time this lane has been paused: the pauses ended, and the pause in progress since when it began.
#[derive(Clone, Default)]
struct PauseClock {
    total: Duration,
    since: Option<Instant>,
}

impl PauseClock {
    /// The time paused up to `now`: the pauses ended, and the pause in progress.
    fn total_at(&self, now: Instant) -> Duration {
        self.total.saturating_add(self.since.map_or(Duration::ZERO, |since| now.saturating_duration_since(since)))
    }

    /// Starts or ends a pause at `now`. A repeat changes nothing.
    fn set(&mut self, paused: bool, now: Instant) {
        match (paused, self.since) {
            (true, None) => self.since = Some(now),
            (false, Some(since)) => {
                self.total = self.total.saturating_add(now.saturating_duration_since(since));
                self.since = None;
            }
            _ => {}
        }
    }
}

/// The perf log's name of a phase's span (`phase.scan`, `phase.import`, ...).
fn span_name(phase: &Phase) -> &'static str {
    match phase {
        Phase::Idle => "phase.idle",
        Phase::Crop => "phase.crop",
        Phase::Scan { .. } => "phase.scan",
        Phase::Import => "phase.import",
        Phase::Hashes => "phase.hashes",
        Phase::Pairing => "phase.pairing",
        Phase::Borders => "phase.borders",
        Phase::Thumbs { .. } => "phase.thumbs",
        Phase::Placeholders => "phase.placeholders",
        Phase::Removing => "phase.removing",
    }
}

/// A lane's own waits: the user's pause of it, and the holds on it ([`Control::hold`]), which count.
#[derive(Default)]
struct Pausing {
    own: bool,
    holds: u32,
}

/// What a bounded checkpoint ([`Control::checkpoint_within`]) found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Checkpoint {
    /// Not paused (any more): go on.
    Go,
    /// A stop was asked for.
    Stop,
    /// Still paused after the wait: the caller does what a long wait needs (gives back what it holds, renews a lease),
    /// and waits again.
    Paused,
}

impl Control {
    /// The root control of a new run.
    pub fn new() -> Control {
        Control::in_family(LaneId::ROOT, Arc::new(Family::new()))
    }

    fn in_family(lane: LaneId, family: Arc<Family>) -> Control {
        Control {
            lane,
            family,
            stop: AtomicBool::new(false),
            paused: Mutex::new(Pausing::default()),
            resumed: Condvar::new(),
            progress: Watch::new(Running::idle()),
            events: Bus::new(EVENT_CAP),
            last_event: Mutex::new(Instant::now()),
            pauses: Mutex::new(PauseClock::default()),
        }
    }

    /// Shares `permits` CPU permits among every lane's workers, for as long as the run lasts (set once: a second call is
    /// ignored). Without it, a worker takes no permit.
    pub fn share_cpu(&self, permits: usize) {
        let _ = self.family.cpu.set(Arc::new(CpuGate::new(permits)));
    }

    /// A worker's CPU permit for the file it processes, when the run shares its CPU ([`Control::share_cpu`]).
    pub fn cpu_permit(&self) -> Option<Permit> {
        self.family.cpu.get().map(|gate| gate.acquire())
    }

    /// The time this lane has been paused, up to now (a pause in progress counts).
    pub fn paused_total(&self) -> Duration {
        let clock = self.pauses.lock().unwrap_or_else(PoisonError::into_inner);
        let open = clock.since.map_or(Duration::ZERO, |since| Instant::now().saturating_duration_since(since));
        clock.total.saturating_add(open)
    }

    /// This control's lane.
    pub fn lane_id(&self) -> LaneId {
        self.lane
    }

    fn is_root(&self) -> bool {
        self.lane == LaneId::ROOT
    }

    /// A lane of this run: a control of its own, with its own stop, pause and progress, sharing the root's global pause,
    /// its stop, the hooks and the tagged bus. Keep the returned `Arc` for as long as the lane runs: the run forgets a
    /// lane once nothing holds it.
    pub fn lane(&self) -> Arc<Control> {
        let id = LaneId(self.family.next_lane.fetch_add(1, Ordering::SeqCst) + 1);
        let lane = Arc::new(Control::in_family(id, self.family.clone()));
        self.family.lanes.lock().unwrap_or_else(PoisonError::into_inner).push(Arc::downgrade(&lane));
        lane
    }

    /// The lanes of the run that are still held, in the order they were made.
    fn members(&self) -> Vec<Arc<Control>> {
        let mut lanes = self.family.lanes.lock().unwrap_or_else(PoisonError::into_inner);
        lanes.retain(|lane| lane.strong_count() > 0);
        lanes.iter().filter_map(Weak::upgrade).collect()
    }

    /// Ask the running pass to stop at the next file; it commits what it has. On the root, every lane stops too.
    pub fn request_stop(&self) {
        if self.is_root() {
            self.family.root_stop.store(true, Ordering::SeqCst);
        } else {
            self.stop.store(true, Ordering::SeqCst);
        }
        self.wake_family();
    }

    /// Clears the stop that [`Control::request_stop`] set: this lane's own, and on the root the family's too.
    pub fn clear_stop(&self) {
        let _own = self.paused.lock().unwrap_or_else(PoisonError::into_inner);
        self.stop.store(false, Ordering::SeqCst);
        if self.is_root() {
            self.family.root_stop.store(false, Ordering::SeqCst);
        }
    }

    /// Whether this lane must stop: its own stop, or the root's.
    pub fn stop_requested(&self) -> bool {
        self.stop.load(Ordering::SeqCst) || self.family.root_stop.load(Ordering::SeqCst)
    }

    /// Whether this lane waits at its checkpoints, given its own pauses: the global pause, its own pause, or a hold.
    fn waits(&self, own: &Pausing) -> bool {
        own.own || own.holds > 0 || self.family.global_paused.load(Ordering::SeqCst)
    }

    /// Pauses or resumes. On the root this is the global pause: every lane waits, and every lane's clock stops. On a lane
    /// it is that lane's pause alone.
    pub fn set_paused(&self, paused: bool) {
        if self.is_root() {
            self.family.global_paused.store(paused, Ordering::SeqCst);
            self.wake_family();
            let now = Instant::now();
            self.refresh_clock(now);
            for lane in self.members() {
                lane.refresh_clock(now);
            }
        } else {
            let mut own = self.paused.lock().unwrap_or_else(PoisonError::into_inner);
            own.own = paused;
            self.resumed.notify_all();
            self.stamp_clock(&own, Instant::now());
        }
        self.changed();
    }

    /// Stops or starts this lane's clock to match whether it waits, read under `own`: the lane's pause lock, which the
    /// caller holds, so the clock is stamped from the state it is applied with.
    fn stamp_clock(&self, own: &Pausing, now: Instant) {
        let paused = self.waits(own);
        self.progress.update(|running| running.set_paused(paused, now));
        self.pauses.lock().unwrap_or_else(PoisonError::into_inner).set(paused, now);
    }

    /// [`Control::stamp_clock`], taking the lane's pause lock itself.
    fn refresh_clock(&self, now: Instant) {
        let own = self.paused.lock().unwrap_or_else(PoisonError::into_inner);
        self.stamp_clock(&own, now);
    }

    /// The hook `changed` calls from now on (set once: a second one is ignored). Shared by the run's lanes.
    pub fn on_change(&self, hook: impl Fn() + Send + Sync + 'static) {
        let _ = self.family.hook.set(Box::new(hook));
    }

    fn changed(&self) {
        if let Some(hook) = self.family.hook.get() {
            hook();
        }
    }

    /// The hook for a change of the count: at most one call per [`NOTICE_INTERVAL`] across the run's lanes. A change the
    /// throttle holds back is told by the next call that passes it, or by the phase's end. The phase's begin and end,
    /// the pauses and the holds call the hook without the throttle.
    fn progress_changed(&self) {
        {
            let mut last = self.family.last_notice.lock().unwrap_or_else(PoisonError::into_inner);
            let now = Instant::now();
            if last.is_some_and(|at| now.saturating_duration_since(at) < NOTICE_INTERVAL) {
                return;
            }
            *last = Some(now);
        }
        self.changed();
    }

    /// The hook [`Control::ledger_changed`] calls from now on (set once: a second one is ignored). Shared by the run's lanes.
    pub fn on_ledger_change(&self, hook: impl Fn() + Send + Sync + 'static) {
        let _ = self.family.on_ledger.set(Box::new(hook));
    }

    /// The pass changed the failure ledger (wrote or cleared rows, or committed a batch whose triggers may have): whoever
    /// shows the ledger reads it again. Called on the pass's thread, after the commit.
    pub fn ledger_changed(&self) {
        if let Some(hook) = self.family.on_ledger.get() {
            hook();
        }
    }

    /// Whether this lane is paused: the global pause, or its own.
    pub fn is_paused(&self) -> bool {
        let own = self.paused.lock().unwrap_or_else(PoisonError::into_inner).own;
        own || self.family.global_paused.load(Ordering::SeqCst)
    }

    /// Whether this lane is paused itself (its own pause, which a per-lane toggle sets), not the run's global pause.
    pub fn is_own_paused(&self) -> bool {
        self.paused.lock().unwrap_or_else(PoisonError::into_inner).own
    }

    /// Called before each unit of work: waits while paused. `false` means stop.
    pub fn checkpoint(&self) -> bool {
        self.checkpoint_until(&AtomicBool::new(false))
    }

    /// [`Control::checkpoint`], which also stops for `stop`, a stop of the caller's own (a pipeline's, set when its
    /// pass ends). Both are checked under the lane's pause lock: whoever sets `stop` and then calls [`Control::wake`]
    /// can't set it between this check and the wait.
    pub fn checkpoint_until(&self, stop: &AtomicBool) -> bool {
        let mut own = self.paused.lock().unwrap_or_else(PoisonError::into_inner);
        loop {
            if self.stop_requested() || stop.load(Ordering::SeqCst) {
                return false;
            }
            if !self.waits(&own) {
                return true;
            }
            own = self.resumed.wait(own).unwrap_or_else(PoisonError::into_inner);
        }
    }

    /// [`Control::checkpoint`], waiting at most `wait` while paused: [`Checkpoint::Paused`] when the pause still holds
    /// then. For a pass that holds something a long pause must not keep (a scan's lease, renewed between waits).
    pub fn checkpoint_within(&self, wait: Duration) -> Checkpoint {
        let deadline = Instant::now() + wait;
        let mut own = self.paused.lock().unwrap_or_else(PoisonError::into_inner);
        loop {
            if self.stop_requested() {
                return Checkpoint::Stop;
            }
            if !self.waits(&own) {
                return Checkpoint::Go;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Checkpoint::Paused;
            }
            own = self.resumed.wait_timeout(own, left).unwrap_or_else(PoisonError::into_inner).0;
        }
    }

    /// Wakes this lane's paused checkpoints to look at their stops again, without resuming: one whose own stop was set
    /// ([`Control::checkpoint_until`]) returns, the others wait on.
    pub fn wake(&self) {
        let _own = self.paused.lock().unwrap_or_else(PoisonError::into_inner);
        self.resumed.notify_all();
    }

    /// [`Control::wake`] for every lane of the run, the root's included.
    fn wake_family(&self) {
        self.wake();
        for lane in self.members() {
            lane.wake();
        }
    }

    /// Holds this lane: it waits at its checkpoints, and [`Control::wait_while_held`] keeps it from starting a round, until
    /// the hold is dropped. Unlike a pause a hold is not the user's: [`Control::is_paused`] says what the user set. Holds
    /// count, and each guard gives back its own. A caller that needs the lane to have stopped asks it to stop as well
    /// ([`Control::request_stop`]), and waits for its round to end.
    pub fn hold(self: &Arc<Self>) -> Hold {
        {
            let mut own = self.paused.lock().unwrap_or_else(PoisonError::into_inner);
            own.holds += 1;
            self.stamp_clock(&own, Instant::now());
        }
        self.changed();
        Hold { ctl: Arc::clone(self) }
    }

    /// Waits while this lane is held. `quit` is asked each time the wait wakes: a quitting lane returns with the hold still
    /// standing. A release and [`Control::wake`] wake the wait.
    pub fn wait_while_held(&self, quit: &dyn Fn() -> bool) {
        let mut own = self.paused.lock().unwrap_or_else(PoisonError::into_inner);
        while own.holds > 0 && !quit() {
            own = self.resumed.wait(own).unwrap_or_else(PoisonError::into_inner);
        }
    }

    /// A phase starts: its progress from 0 of `total`. It ends when the guard is dropped, however the pass leaves (a
    /// return, an error, a panic): the final progress goes out as [`Event::Finished`], and the phase is idle.
    pub fn begin(&self, phase: Phase, total: u64) -> PhaseGuard<'_> {
        let now = Instant::now();
        *self.last_event.lock().unwrap_or_else(PoisonError::into_inner) = now;
        // The phase's span in the perf log: opened here, closed with the guard (inert without a perf sink).
        let span = pg_perf::span(span_name(&phase));
        let progress = Progress { phase, total, ..Progress::idle() };
        // The pauses and holds outlive phases: a phase that begins while paused starts with its clock stopped.
        let mut running = Running::begun(progress, now);
        {
            let own = self.paused.lock().unwrap_or_else(PoisonError::into_inner);
            running.set_paused(self.waits(&own), now);
            self.progress.set(running);
        }
        self.changed();
        PhaseGuard { ctl: self, span }
    }

    /// One file done.
    pub fn add(&self, bytes: usize, failed: bool) {
        self.update(|p| {
            p.done += 1;
            p.failed += u64::from(failed);
            p.bytes += count_u64(bytes);
        });
    }

    /// Set the count directly (the scanner counts images found).
    pub fn set_done(&self, done: u64) {
        self.update(|p| p.done = done);
    }

    fn update(&self, change: impl FnOnce(&mut Progress)) {
        let mut last = self.last_event.lock().unwrap_or_else(PoisonError::into_inner);
        let due = self.progress.update(|running| {
            let now = Instant::now();
            running.change(now, change);
            (now.saturating_duration_since(*last) >= EVENT_INTERVAL).then(|| running.read(now))
        });
        if let Some(progress) = due {
            *last = Instant::now();
            self.publish(Event::Progress(progress));
        }
        self.progress_changed();
    }

    /// Sends an event to this lane's subscribers and to the run's tagged bus.
    fn publish(&self, event: Event) {
        self.family.everything.publish(Tagged { lane: self.lane, event: event.clone() });
        self.events.publish(event);
    }

    /// The phase ended: its final progress goes out as an event, and the phase is idle. Called by [`PhaseGuard`]'s drop.
    /// Returns that progress, and the time the phase was paused (its wall time less its working time).
    fn finish(&self) -> (Progress, Duration) {
        let (ended, paused) = self.progress.update(|running| {
            let now = Instant::now();
            let wall = now.saturating_duration_since(running.started);
            let ended = std::mem::replace(running, Running::ended(now)).read(now);
            let paused = wall.saturating_sub(ended.elapsed);
            (ended, paused)
        });
        self.publish(Event::Finished(ended.clone()));
        self.changed();
        (ended, paused)
    }

    /// The progress now, with the time its phase has run.
    pub fn snapshot(&self) -> Progress {
        self.progress.get().read(Instant::now())
    }

    /// The progress now, and the time since work last ran (`Running::since_work`), read together so the two agree.
    pub fn snapshot_since(&self) -> (Progress, Option<Duration>) {
        let now = Instant::now();
        let running = self.progress.get();
        (running.read(now), running.since_work(now))
    }

    /// Tells the hook that something changed that no progress event reports (a lane's label, a retired lane).
    pub fn announce(&self) {
        self.changed();
    }

    /// Every lane's progress now, the root's and the held lanes', in lane order. Called on the root.
    pub fn snapshots(&self) -> Vec<(LaneId, Progress)> {
        let mut out = vec![(self.lane, self.snapshot())];
        out.extend(self.members().iter().map(|lane| (lane.lane, lane.snapshot())));
        out.sort_by_key(|(id, _)| *id);
        out
    }

    /// Progress and phase-end events of this lane from now on.
    pub fn subscribe(&self) -> Subscription<Event> {
        self.events.subscribe()
    }

    /// Progress and phase-end events of every lane from now on, each tagged with its lane (the root's included).
    pub fn subscribe_all(&self) -> Subscription<Tagged> {
        self.family.everything.subscribe()
    }
}

impl Default for Control {
    fn default() -> Control {
        Control::new()
    }
}

/// A hold on a lane ([`Control::hold`]): the lane waits until it is dropped.
pub struct Hold {
    ctl: Arc<Control>,
}

impl Drop for Hold {
    fn drop(&mut self) {
        {
            let mut own = self.ctl.paused.lock().unwrap_or_else(PoisonError::into_inner);
            own.holds = own.holds.saturating_sub(1);
            self.ctl.resumed.notify_all();
            self.ctl.stamp_clock(&own, Instant::now());
        }
        self.ctl.changed();
    }
}

/// A phase in progress ([`Control::begin`]); dropping it ends the phase.
#[must_use = "the phase ends when the guard is dropped: bind it (`let _phase = ctl.begin(..)`) for the pass's length"]
pub struct PhaseGuard<'a> {
    ctl: &'a Control,
    /// The phase's span in the perf log: what the phase did, and its pauses, go on it when the guard drops.
    span: pg_perf::Span,
}

impl Drop for PhaseGuard<'_> {
    fn drop(&mut self) {
        let (ended, paused) = self.ctl.finish();
        self.span.items(ended.done);
        self.span.failed(ended.failed);
        self.span.bytes(ended.bytes);
        self.span.field("total", ended.total);
        self.span.paused(paused);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::*;

    /// The second a ten-minute pause ends at, in the clock tests that take their times from `at`: it starts at the tenth
    /// second, after ten files a second for ten seconds.
    const PAUSE_ENDS: u64 = 610;

    #[test]
    fn a_progress_read_has_the_time_its_phase_has_run() {
        let ctl = Control::new();
        let _phase = ctl.begin(Phase::Import, 4);
        std::thread::sleep(Duration::from_millis(20));
        let first = ctl.snapshot();
        assert!(first.elapsed >= Duration::from_millis(20), "{:?}", first.elapsed);
        std::thread::sleep(Duration::from_millis(5));
        assert!(ctl.snapshot().elapsed > first.elapsed, "read again, later");
    }

    #[test]
    fn the_rate_is_over_the_latest_window_and_the_marks_stay_few() {
        let t0 = Instant::now();
        let mut running = Running::begun(Progress { total: 1000, ..Progress::idle() }, t0);
        for s in 1..=60u64 {
            // 10 files a second for the first 30 s, then 2 a second.
            running.progress.done = if s <= 30 { s * 10 } else { 300 + (s - 30) * 2 };
            running.mark(t0 + Duration::from_secs(s));
        }
        let read = running.read(t0 + Duration::from_secs(60));
        assert_eq!(read.elapsed, Duration::from_secs(60));
        assert_eq!(read.rate(), 2.0, "the last 30 s: 60 files");
        assert_eq!(read.eta(), 320.0, "640 files left at 2 a second");
        assert!(running.marks.len() <= 32, "{} marks", running.marks.len());
    }

    #[test]
    fn early_in_a_phase_the_window_is_the_whole_phase() {
        let t0 = Instant::now();
        let mut running = Running::begun(Progress { total: 100, ..Progress::idle() }, t0);
        running.progress.done = 50;
        running.mark(t0 + Duration::from_secs(5));
        assert_eq!(running.read(t0 + Duration::from_secs(5)).rate(), 10.0);
    }

    #[test]
    fn a_pause_stops_the_clock_and_the_rate_is_the_same_after_it() {
        let t0 = Instant::now();
        let at = |s: u64| t0 + Duration::from_secs(s);
        let mut running = Running::begun(Progress { total: 1000, ..Progress::idle() }, t0);
        // 10 files a second for 10 s, then paused for 10 minutes with nothing finished.
        for s in 1..=10 {
            running.change(at(s), |p| p.done += 10);
        }
        let before = running.read(at(10));
        running.set_paused(true, at(10));
        let during = running.read(at(PAUSE_ENDS));
        assert_eq!(during.elapsed, Duration::from_secs(10), "the clock stands still");
        assert_eq!(during.eta(), before.eta(), "the time left doesn't climb during a pause");
        running.set_paused(false, at(PAUSE_ENDS));
        // 10 files a second for 5 s after the pause.
        for s in 1..=5 {
            running.change(at(PAUSE_ENDS + s), |p| p.done += 10);
        }
        let after = running.read(at(615));
        assert_eq!(after.elapsed, Duration::from_secs(15));
        assert_eq!(after.rate(), 10.0, "the pause took no working time");
    }

    #[test]
    fn files_that_finish_while_paused_are_not_in_the_rate() {
        let t0 = Instant::now();
        let at = |s: u64| t0 + Duration::from_secs(s);
        let mut running = Running::begun(Progress { total: 1000, ..Progress::idle() }, t0);
        for s in 1..=10 {
            running.change(at(s), |p| p.done += 10);
        }
        running.set_paused(true, at(10));
        // Files already past their checkpoint still finish while paused.
        for _ in 0..40 {
            running.change(at(10), |p| p.done += 1);
        }
        running.set_paused(false, at(PAUSE_ENDS));
        let read = running.read(at(PAUSE_ENDS));
        assert_eq!(read.done, 140);
        assert_eq!(read.rate(), 10.0, "100 files in 10 s of working time, not 140");
    }

    #[test]
    fn a_repeated_pause_or_resume_changes_nothing() {
        let t0 = Instant::now();
        let at = |s: u64| t0 + Duration::from_secs(s);
        let mut running = Running::begun(Progress::idle(), t0);
        running.set_paused(true, at(5));
        running.set_paused(true, at(7));
        running.set_paused(false, at(10));
        running.set_paused(false, at(20));
        assert_eq!(running.active(at(20)), Duration::from_secs(15), "only the first pause counts");
    }

    #[test]
    fn a_phase_that_begins_while_paused_starts_its_clock_at_the_resume() {
        let ctl = Control::new();
        ctl.set_paused(true);
        let _phase = ctl.begin(Phase::Import, 4);
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(ctl.snapshot().elapsed, Duration::ZERO);
        ctl.set_paused(false);
        std::thread::sleep(Duration::from_millis(20));
        assert!(ctl.snapshot().elapsed >= Duration::from_millis(20));
    }

    #[test]
    fn a_paused_checkpoint_waits_for_the_resume() {
        let ctl = Arc::new(Control::new());
        ctl.set_paused(true);
        let (tx, rx) = mpsc::channel();
        let waiting = ctl.clone();
        std::thread::spawn(move || tx.send(waiting.checkpoint()).unwrap());
        assert!(rx.recv_timeout(Duration::from_millis(50)).is_err());
        ctl.set_paused(false);
        assert!(rx.recv_timeout(Duration::from_secs(5)).unwrap());
    }

    #[test]
    fn a_hold_stops_a_lane_at_its_checkpoints_but_is_not_the_users_pause() {
        let root = Control::new();
        let lane = root.lane();
        let hold = lane.hold();
        assert!(!lane.is_paused(), "a hold is not the user's pause");
        let (sent, got) = std::sync::mpsc::channel();
        let waiter = Arc::clone(&lane);
        std::thread::spawn(move || {
            sent.send(waiter.checkpoint()).ok();
        });
        assert!(got.recv_timeout(Duration::from_millis(100)).is_err(), "a held checkpoint waits");
        drop(hold);
        assert_eq!(got.recv_timeout(Duration::from_secs(5)), Ok(true), "the release lets it go on");
    }

    #[test]
    fn a_quit_ends_the_wait_for_a_hold_and_the_hold_still_stands() {
        let root = Control::new();
        let lane = root.lane();
        let hold = lane.hold();
        let quit = AtomicBool::new(true);
        lane.wait_while_held(&|| quit.load(Ordering::SeqCst));
        assert_eq!(lane.checkpoint_within(Duration::ZERO), Checkpoint::Paused, "the hold still stands");
        drop(hold);
        assert_eq!(lane.checkpoint_within(Duration::ZERO), Checkpoint::Go);
    }

    #[test]
    fn a_stop_wakes_a_paused_checkpoint() {
        let ctl = Arc::new(Control::new());
        ctl.set_paused(true);
        let (tx, rx) = mpsc::channel();
        let waiting = ctl.clone();
        std::thread::spawn(move || tx.send(waiting.checkpoint()).unwrap());
        assert!(rx.recv_timeout(Duration::from_millis(50)).is_err());
        ctl.request_stop();
        assert!(!rx.recv_timeout(Duration::from_secs(5)).unwrap());
    }

    #[test]
    fn waking_a_paused_checkpoint_stops_only_the_one_whose_own_stop_is_set() {
        let ctl = Arc::new(Control::new());
        ctl.set_paused(true);
        let stop = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let (own, other) = (tx.clone(), tx);
        let (waiting, flag) = (ctl.clone(), stop.clone());
        std::thread::spawn(move || own.send(("own", waiting.checkpoint_until(&flag))).unwrap());
        let waiting = ctl.clone();
        std::thread::spawn(move || other.send(("other", waiting.checkpoint())).unwrap());
        assert!(rx.recv_timeout(Duration::from_millis(50)).is_err());
        stop.store(true, Ordering::SeqCst);
        ctl.wake();
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), ("own", false));
        // The other is still paused: the wake is not a resume.
        assert!(rx.recv_timeout(Duration::from_millis(50)).is_err());
        ctl.set_paused(false);
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), ("other", true));
    }

    #[test]
    fn a_bounded_checkpoint_says_paused_after_its_wait_and_goes_or_stops_at_once() {
        let ctl = Control::new();
        assert_eq!(ctl.checkpoint_within(Duration::from_secs(5)), Checkpoint::Go);
        ctl.set_paused(true);
        let began = Instant::now();
        assert_eq!(ctl.checkpoint_within(Duration::from_millis(30)), Checkpoint::Paused);
        assert!(began.elapsed() >= Duration::from_millis(30), "{:?}", began.elapsed());
        ctl.request_stop();
        assert_eq!(ctl.checkpoint_within(Duration::from_secs(5)), Checkpoint::Stop);
    }

    #[test]
    fn a_ledger_change_calls_its_hook_and_nothing_without_one() {
        let ctl = Control::new();
        ctl.ledger_changed();
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = calls.clone();
        ctl.on_ledger_change(move || {
            counted.fetch_add(1, Ordering::SeqCst);
        });
        ctl.ledger_changed();
        ctl.ledger_changed();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_burst_of_file_changes_reaches_the_hook_a_few_times_and_the_phase_edges_always_do() {
        let ctl = Control::new();
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = calls.clone();
        ctl.on_change(move || {
            counted.fetch_add(1, Ordering::SeqCst);
        });
        let phase = ctl.begin(Phase::Import, 1000);
        assert_eq!(calls.load(Ordering::SeqCst), 1, "the phase's begin always reaches the hook");
        for _ in 0..1000 {
            ctl.add(1, false);
        }
        let during = calls.load(Ordering::SeqCst);
        assert!(during <= 3, "a burst within the notice interval is a notice or two, not {during}");
        drop(phase);
        assert_eq!(calls.load(Ordering::SeqCst), during + 1, "the phase's end always reaches the hook");
    }

    #[test]
    fn a_dropped_phase_guard_publishes_the_last_progress_and_goes_idle() {
        let ctl = Control::new();
        let sub = ctl.subscribe();
        let phase = ctl.begin(Phase::Import, 10);
        ctl.add(5, false);
        ctl.add(7, true);
        assert_eq!(ctl.snapshot().phase, Phase::Import);
        drop(phase);
        let items = sub.take().items;
        match items.as_slice() {
            [Event::Finished(p)] => {
                assert_eq!((p.phase.clone(), p.done, p.failed, p.bytes), (Phase::Import, 2, 1, 12));
            }
            other => panic!("expected one Finished event, got {} events", other.len()),
        }
        assert_eq!(ctl.snapshot().phase, Phase::Idle);
    }

    #[test]
    fn a_pass_that_fails_or_panics_still_ends_its_phase() {
        let ctl = Control::new();
        let pass = |fail: bool| -> Result<(), &'static str> {
            let _phase = ctl.begin(Phase::Borders, 3);
            if fail {
                return Err("the pass failed");
            }
            Ok(())
        };
        assert!(pass(true).is_err());
        assert_eq!(ctl.snapshot().phase, Phase::Idle);
        let panicked = crate::panics::catch(std::panic::AssertUnwindSafe(|| {
            let _phase = ctl.begin(Phase::Crop, 1);
            panic!("the pass panicked");
        }));
        assert!(panicked.is_err());
        assert_eq!(ctl.snapshot().phase, Phase::Idle);
    }

    #[test]
    fn since_work_is_none_before_a_phase_ends_then_zero_while_one_runs_then_grows() {
        let ctl = Control::new();
        assert_eq!(ctl.snapshot_since().1, None, "no phase has ended yet");
        let phase = ctl.begin(Phase::Import, 10);
        assert_eq!(ctl.snapshot_since().1, Some(Duration::ZERO), "a phase runs");
        drop(phase);
        std::thread::sleep(Duration::from_millis(20));
        let (p, since) = ctl.snapshot_since();
        assert_eq!(p.phase, Phase::Idle);
        assert!(since.is_some_and(|s| s >= Duration::from_millis(20)), "the time since the phase ended grows");
        let _again = ctl.begin(Phase::Hashes, 1);
        assert_eq!(ctl.snapshot_since().1, Some(Duration::ZERO), "a new phase resets it");
    }

    #[test]
    fn since_work_ends_with_a_panicking_phase_and_a_lane_keeps_its_own() {
        let root = Control::new();
        let (a, b) = (root.lane(), root.lane());
        let panicked = crate::panics::catch(std::panic::AssertUnwindSafe(|| {
            let _phase = a.begin(Phase::Import, 1);
            panic!("the pass panicked");
        }));
        assert!(panicked.is_err());
        assert!(a.snapshot_since().1.is_some(), "the panic still ended the phase");
        assert_eq!(b.snapshot_since().1, None, "a sibling that never worked has no time since work");
    }

    #[test]
    fn a_lane_pause_leaves_its_siblings_running() {
        let root = Control::new();
        let (a, b) = (root.lane(), root.lane());
        a.set_paused(true);
        assert!(a.is_paused());
        assert!(!b.is_paused(), "a sibling is not paused");
        let (tx, rx) = mpsc::channel();
        let waiting = a.clone();
        std::thread::spawn(move || tx.send(waiting.checkpoint()).unwrap());
        assert!(rx.recv_timeout(Duration::from_millis(50)).is_err(), "the paused lane waits");
        assert!(b.checkpoint(), "the sibling goes on");
        a.set_paused(false);
        assert!(rx.recv_timeout(Duration::from_secs(5)).unwrap());
    }

    #[test]
    fn a_global_pause_reaches_every_lane_and_stops_their_clocks() {
        let root = Control::new();
        let lane = root.lane();
        let _phase = lane.begin(Phase::Import, 4);
        root.set_paused(true);
        assert!(lane.is_paused(), "the lane waits for the global pause");
        let before = lane.snapshot().elapsed;
        std::thread::sleep(Duration::from_millis(30));
        assert_eq!(lane.snapshot().elapsed, before, "the lane's clock stands still");
        root.set_paused(false);
        assert!(!lane.is_paused());
    }

    #[test]
    fn a_lane_stop_leaves_its_siblings_and_the_root_stop_reaches_every_lane() {
        let root = Control::new();
        let (a, b) = (root.lane(), root.lane());
        a.request_stop();
        assert!(a.stop_requested());
        assert!(!b.stop_requested(), "a sibling keeps going");
        root.request_stop();
        assert!(b.stop_requested(), "the root's stop reaches every lane");
        assert!(!a.checkpoint(), "a stopped lane's checkpoint says stop");
    }

    #[test]
    fn subscribe_all_tags_each_lanes_events() {
        let root = Control::new();
        let sub = root.subscribe_all();
        let lane = root.lane();
        {
            let _phase = lane.begin(Phase::Import, 1);
            lane.add(3, false);
        }
        let items = sub.take().items;
        assert!(items.iter().all(|t| t.lane == lane.lane_id()), "{items:?}");
        assert!(matches!(items.last().map(|t| &t.event), Some(Event::Finished(_))));
    }

    #[test]
    fn snapshots_lists_the_root_and_its_held_lanes_in_lane_order() {
        let root = Control::new();
        let first = root.lane();
        let second = root.lane();
        let lanes: Vec<LaneId> = root.snapshots().into_iter().map(|(id, _)| id).collect();
        assert_eq!(lanes, vec![LaneId::ROOT, first.lane_id(), second.lane_id()]);
        drop(second);
        assert_eq!(root.snapshots().len(), 2, "a lane nothing holds is forgotten");
    }
}
