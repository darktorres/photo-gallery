//! What a running pass shares with whoever started it: its progress, pause, and a stop request (a new job, a new
//! thumbnail height, quitting). It holds no text: whoever shows the progress formats it (the command line, the app).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use crate::events::{Bus, Subscription, Watch};

/// The phase a pass is in, and what it works on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Phase {
    Idle,
    Crop,
    /// The folder being scanned.
    Scan {
        root: String,
    },
    Import,
    Borders,
    /// The thumbnails made at this height.
    Thumbs {
        height: u32,
    },
}

#[derive(Clone, Debug)]
pub struct Progress {
    pub phase: Phase,
    pub done: u64,
    /// 0 when unknown (scanning).
    pub total: u64,
    pub failed: u64,
    pub bytes: u64,
    pub started: Instant,
}

impl Progress {
    fn idle() -> Progress {
        Progress { phase: Phase::Idle, done: 0, total: 0, failed: 0, bytes: 0, started: Instant::now() }
    }

    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    /// Files per second so far.
    pub fn rate(&self) -> f64 {
        count(self.done) / self.elapsed().as_secs_f64().max(1e-3)
    }

    /// Seconds left at the rate so far (0 when there is no rate yet or nothing is known).
    pub fn eta(&self) -> f64 {
        let rate = self.rate();
        if rate > 0.0 { count(self.total.saturating_sub(self.done)) / rate } else { 0.0 }
    }

    /// Done over total, 0 when the total is unknown.
    pub fn fraction(&self) -> f64 {
        if self.total == 0 { 0.0 } else { count(self.done) / count(self.total) }
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
    progress: Watch<Progress>,
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
            progress: Watch::new(Progress::idle()),
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
        self.progress.set(Progress { phase, done: 0, total, failed: 0, bytes: 0, started: now });
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
        let due = self.progress.update(|p| {
            change(p);
            (last.elapsed() >= EVENT_INTERVAL).then(|| p.clone())
        });
        if let Some(progress) = due {
            *last = Instant::now();
            self.events.publish(Event::Progress(progress));
        }
        self.changed();
    }

    /// The phase ended: its final progress goes out as an event, and the phase is idle. Called by [`PhaseGuard`]'s drop.
    fn finish(&self) {
        let ended = self.progress.update(|p| std::mem::replace(p, Progress::idle()));
        self.events.publish(Event::Finished(ended));
        self.changed();
    }

    pub fn snapshot(&self) -> Progress {
        self.progress.get()
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

/// A count of a job's items as an `f64`: exact up to 2^53, which no job reaches.
#[expect(
    clippy::as_conversions,
    clippy::cast_precision_loss,
    reason = "an f64 holds integers exactly up to 2^53, which no job reaches"
)]
fn count(x: u64) -> f64 {
    x as f64
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::mpsc;

    use super::*;

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
