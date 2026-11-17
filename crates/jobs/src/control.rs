//! What a running pass shares with whoever started it: its progress, pause, and a stop request (a new job, a new
//! thumbnail height, quitting). It holds no text: whoever shows the progress formats it (the command line, the app).
//! The phase and the progress are the model's (`pg_model::progress`); the clock they are timed by is here.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use pg_model::progress::{Mark, Phase, Progress};

use crate::events::{Bus, Subscription, Watch};

/// The rate and the time left are taken over this much of the phase's latest progress: a sliding window.
const RATE_WINDOW: Duration = Duration::from_secs(30);
/// Marks of the count go down at most this often, so a window holds about `RATE_WINDOW / MARK_EVERY` of them.
const MARK_EVERY: Duration = Duration::from_millis(500);

/// The progress of the phase running, when it began, and the marks of its count the rate window is taken from.
#[derive(Clone)]
struct Running {
    progress: Progress,
    started: Instant,
    /// The time the pass has spent paused, up to the latest resume.
    paused_total: Duration,
    /// When the current pause began, while paused.
    paused_since: Option<Instant>,
    /// Files that finished while paused. They took no working time, so the rate leaves them out.
    drained: u64,
    /// (working time, count without the drained files), oldest first. The front one is where the window starts: the
    /// latest mark at least a window old, or the phase's start.
    marks: VecDeque<(Duration, u64)>,
}

impl Running {
    fn begun(progress: Progress, started: Instant) -> Running {
        Running {
            progress,
            started,
            paused_total: Duration::ZERO,
            paused_since: None,
            drained: 0,
            marks: VecDeque::from([(Duration::ZERO, 0)]),
        }
    }

    fn idle() -> Running {
        Running::begun(Progress::idle(), Instant::now())
    }

    /// The time the phase has run at `now`, not counting its pauses.
    fn active(&self, now: Instant) -> Duration {
        let paused = self.paused_since.map_or(Duration::ZERO, |since| now.saturating_duration_since(since));
        now.saturating_duration_since(self.started).saturating_sub(self.paused_total.saturating_add(paused))
    }

    /// Pauses or resumes the clock at `now`. A repeat changes nothing.
    fn set_paused(&mut self, paused: bool, now: Instant) {
        match (paused, self.paused_since) {
            (true, None) => self.paused_since = Some(now),
            (false, Some(since)) => {
                self.paused_total = self.paused_total.saturating_add(now.saturating_duration_since(since));
                self.paused_since = None;
            }
            _ => {}
        }
    }

    /// Applies a change of the progress at `now`. A file finished while paused is drained, not counted in the window.
    fn change(&mut self, now: Instant, change: impl FnOnce(&mut Progress)) {
        let before = self.progress.done;
        change(&mut self.progress);
        if self.paused_since.is_some() {
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
    Progress(Progress),
    Finished(Progress),
}

/// Progress events come at most this often while a phase runs.
const EVENT_INTERVAL: Duration = Duration::from_secs(5);
/// Events a subscriber can fall behind by before the oldest are dropped.
const EVENT_CAP: usize = 64;

pub struct Control {
    stop: AtomicBool,
    /// The pause flag. The lock also orders a stop against a checkpoint's check, so a stop can't be missed while the
    /// checkpoint waits on `resumed`.
    paused: Mutex<bool>,
    resumed: Condvar,
    progress: Watch<Running>,
    events: Bus<Event>,
    /// Called whenever what `snapshot` or `is_paused` says changes (progress, the pause), for an owner that
    /// reads them on its own thread instead of polling.
    hook: OnceLock<Box<dyn Fn() + Send + Sync>>,
    /// Called when the pass changed the failure ledger ([`Control::ledger_changed`]): the runner pokes the inbox, which
    /// reads the ledger again.
    on_ledger: OnceLock<Box<dyn Fn() + Send + Sync>>,
    /// When the last progress event went out.
    last_event: Mutex<Instant>,
    /// The pauses since the job started, for the perf log's pass records (a pass's pauses outside its phases count too).
    pauses: Mutex<PauseClock>,
}

/// The time the job has been paused: the pauses ended, and the pause in progress since when it began.
#[derive(Default)]
struct PauseClock {
    total: Duration,
    since: Option<Instant>,
}

impl PauseClock {
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
    }
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
    pub fn new() -> Control {
        Control {
            stop: AtomicBool::new(false),
            paused: Mutex::new(false),
            resumed: Condvar::new(),
            progress: Watch::new(Running::idle()),
            events: Bus::new(EVENT_CAP),
            hook: OnceLock::new(),
            on_ledger: OnceLock::new(),
            last_event: Mutex::new(Instant::now()),
            pauses: Mutex::new(PauseClock::default()),
        }
    }

    /// The time the job has been paused, up to now (a pause in progress counts).
    pub fn paused_total(&self) -> Duration {
        let clock = self.pauses.lock().unwrap_or_else(PoisonError::into_inner);
        let open = clock.since.map_or(Duration::ZERO, |since| Instant::now().saturating_duration_since(since));
        clock.total.saturating_add(open)
    }

    /// Ask the running pass to stop at the next file; it commits what it has.
    pub fn request_stop(&self) {
        let _paused = self.paused.lock().unwrap_or_else(PoisonError::into_inner);
        self.stop.store(true, Ordering::SeqCst);
        self.resumed.notify_all();
    }

    pub fn clear_stop(&self) {
        let _paused = self.paused.lock().unwrap_or_else(PoisonError::into_inner);
        self.stop.store(false, Ordering::SeqCst);
    }

    pub fn stop_requested(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }

    pub fn set_paused(&self, paused: bool) {
        {
            let mut guard = self.paused.lock().unwrap_or_else(PoisonError::into_inner);
            *guard = paused;
            let now = Instant::now();
            self.progress.update(|running| running.set_paused(paused, now));
            self.pauses.lock().unwrap_or_else(PoisonError::into_inner).set(paused, now);
            self.resumed.notify_all();
        }
        self.changed();
    }

    /// The hook `changed` calls from now on (set once: a second one is ignored).
    pub fn on_change(&self, hook: impl Fn() + Send + Sync + 'static) {
        let _ = self.hook.set(Box::new(hook));
    }

    fn changed(&self) {
        if let Some(hook) = self.hook.get() {
            hook();
        }
    }

    /// The hook [`Control::ledger_changed`] calls from now on (set once: a second one is ignored).
    pub fn on_ledger_change(&self, hook: impl Fn() + Send + Sync + 'static) {
        let _ = self.on_ledger.set(Box::new(hook));
    }

    /// The pass changed the failure ledger (wrote or cleared rows, or committed a batch whose triggers may have): whoever
    /// shows the ledger reads it again. Called on the pass's thread, after the commit.
    pub fn ledger_changed(&self) {
        if let Some(hook) = self.on_ledger.get() {
            hook();
        }
    }

    pub fn is_paused(&self) -> bool {
        *self.paused.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Called before each unit of work: waits while paused. `false` means stop.
    pub fn checkpoint(&self) -> bool {
        self.checkpoint_until(&AtomicBool::new(false))
    }

    /// [`Control::checkpoint`], which also stops for `stop`, a stop of the caller's own (a pipeline's, set when its
    /// pass ends). Both are checked under the pause lock: whoever sets `stop` and then calls [`Control::wake`] can't
    /// set it between this check and the wait.
    pub fn checkpoint_until(&self, stop: &AtomicBool) -> bool {
        let mut paused = self.paused.lock().unwrap_or_else(PoisonError::into_inner);
        loop {
            if self.stop_requested() || stop.load(Ordering::SeqCst) {
                return false;
            }
            if !*paused {
                return true;
            }
            paused = self.resumed.wait(paused).unwrap_or_else(PoisonError::into_inner);
        }
    }

    /// [`Control::checkpoint`], waiting at most `wait` while paused: [`Checkpoint::Paused`] when the pause still holds
    /// then. For a pass that holds something a long pause must not keep (a scan's lease, renewed between waits).
    pub fn checkpoint_within(&self, wait: Duration) -> Checkpoint {
        let deadline = Instant::now() + wait;
        let mut paused = self.paused.lock().unwrap_or_else(PoisonError::into_inner);
        loop {
            if self.stop_requested() {
                return Checkpoint::Stop;
            }
            if !*paused {
                return Checkpoint::Go;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Checkpoint::Paused;
            }
            paused = self.resumed.wait_timeout(paused, left).unwrap_or_else(PoisonError::into_inner).0;
        }
    }

    /// Wakes every paused checkpoint to look at its stops again, without resuming: one whose own stop was set
    /// ([`Control::checkpoint_until`]) returns, the others wait on.
    pub fn wake(&self) {
        let _paused = self.paused.lock().unwrap_or_else(PoisonError::into_inner);
        self.resumed.notify_all();
    }

    /// A phase starts: its progress from 0 of `total`. It ends when the guard is dropped, however the pass leaves (a
    /// return, an error, a panic): the final progress goes out as [`Event::Finished`], and the phase is idle.
    pub fn begin(&self, phase: Phase, total: u64) -> PhaseGuard<'_> {
        let now = Instant::now();
        *self.last_event.lock().unwrap_or_else(PoisonError::into_inner) = now;
        // The phase's span in the perf log: opened here, closed with the guard (inert without a perf sink).
        let span = pg_perf::span(span_name(&phase));
        let progress = Progress { phase, total, ..Progress::idle() };
        {
            // The pause flag outlives phases: a phase that begins while paused starts with its clock stopped.
            let paused = self.paused.lock().unwrap_or_else(PoisonError::into_inner);
            let mut running = Running::begun(progress, now);
            running.set_paused(*paused, now);
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
            p.bytes += u64::try_from(bytes).unwrap_or(u64::MAX);
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
            self.events.publish(Event::Progress(progress));
        }
        self.changed();
    }

    /// The phase ended: its final progress goes out as an event, and the phase is idle. Called by [`PhaseGuard`]'s drop.
    /// Returns that progress, and the time the phase was paused (its wall time less its working time).
    fn finish(&self) -> (Progress, Duration) {
        let (ended, paused) = self.progress.update(|running| {
            let now = Instant::now();
            let wall = now.saturating_duration_since(running.started);
            let ended = std::mem::replace(running, Running::idle()).read(now);
            let paused = wall.saturating_sub(ended.elapsed);
            (ended, paused)
        });
        self.events.publish(Event::Finished(ended.clone()));
        self.changed();
        (ended, paused)
    }

    /// The progress now, with the time its phase has run.
    pub fn snapshot(&self) -> Progress {
        self.progress.get().read(Instant::now())
    }

    /// Progress and phase-end events from now on.
    pub fn subscribe(&self) -> Subscription<Event> {
        self.events.subscribe()
    }
}

impl Default for Control {
    fn default() -> Control {
        Control::new()
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
    use std::sync::Arc;
    use std::sync::mpsc;

    use super::*;

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
        let during = running.read(at(610));
        assert_eq!(during.elapsed, Duration::from_secs(10), "the clock stands still");
        assert_eq!(during.eta(), before.eta(), "the time left doesn't climb during a pause");
        running.set_paused(false, at(610));
        // 10 files a second for 5 s after the pause.
        for s in 1..=5 {
            running.change(at(610 + s), |p| p.done += 10);
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
        running.set_paused(false, at(610));
        let read = running.read(at(610));
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
        let pass = |fail: bool| -> anyhow::Result<()> {
            let _phase = ctl.begin(Phase::Borders, 3);
            anyhow::ensure!(!fail, "the pass failed");
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
}
