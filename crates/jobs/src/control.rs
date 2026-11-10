//! What a running pass shares with whoever started it: its progress, pause, and a stop request (a new job, a new
//! thumbnail height, quitting). It holds no text: whoever shows the progress formats it (the command line, the app).
//! The phase and the progress are the model's (`pg_model::progress`); the clock they are timed by is here.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use pg_model::progress::{Phase, Progress};

use crate::events::{Bus, Subscription, Watch};

/// The progress of the phase running, and when it began: a [`Progress`] read from it has the time since then.
#[derive(Clone)]
struct Running {
    progress: Progress,
    started: Instant,
}

impl Running {
    fn idle() -> Running {
        Running { progress: Progress::idle(), started: Instant::now() }
    }

    /// The progress as it is now, with the time the phase has run.
    fn read(&self) -> Progress {
        Progress { elapsed: self.started.elapsed(), ..self.progress.clone() }
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
    /// When the last progress event went out.
    last_event: Mutex<Instant>,
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
            last_event: Mutex::new(Instant::now()),
        }
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
        let progress = Progress { phase, total, ..Progress::idle() };
        self.progress.set(Running { progress, started: now });
        self.changed();
        PhaseGuard { ctl: self }
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
            change(&mut running.progress);
            (last.elapsed() >= EVENT_INTERVAL).then(|| running.read())
        });
        if let Some(progress) = due {
            *last = Instant::now();
            self.events.publish(Event::Progress(progress));
        }
        self.changed();
    }

    /// The phase ended: its final progress goes out as an event, and the phase is idle. Called by [`PhaseGuard`]'s drop.
    fn finish(&self) {
        let ended = self.progress.update(|running| std::mem::replace(running, Running::idle()).read());
        self.events.publish(Event::Finished(ended));
        self.changed();
    }

    /// The progress now, with the time its phase has run.
    pub fn snapshot(&self) -> Progress {
        self.progress.get().read()
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
}

impl Drop for PhaseGuard<'_> {
    fn drop(&mut self) {
        self.ctl.finish();
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
