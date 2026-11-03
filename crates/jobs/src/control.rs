//! What a running pass shares with whoever started it: its progress, pause, and a stop request (a new job, a new
//! thumbnail height, quitting). It holds no text: whoever shows the progress formats it (the command line, the app).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex};
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
        self.done as f64 / self.elapsed().as_secs_f64().max(1e-3)
    }

    /// Seconds left at the rate so far (0 when there is no rate yet or nothing is known).
    pub fn eta(&self) -> f64 {
        let rate = self.rate();
        if rate > 0.0 { self.total.saturating_sub(self.done) as f64 / rate } else { 0.0 }
    }

    /// Done over total, 0 when the total is unknown.
    pub fn fraction(&self) -> f64 {
        if self.total == 0 { 0.0 } else { self.done as f64 / self.total as f64 }
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
            last_event: Mutex::new(Instant::now()),
        }
    }

    /// Ask the running pass to stop at the next file; it commits what it has.
    pub fn request_stop(&self) {
        let _paused = self.paused.lock().unwrap();
        self.stop.store(true, Ordering::SeqCst);
        self.resumed.notify_all();
    }

    pub fn clear_stop(&self) {
        let _paused = self.paused.lock().unwrap();
        self.stop.store(false, Ordering::SeqCst);
    }

    pub fn stop_requested(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }

    pub fn set_paused(&self, paused: bool) {
        let mut guard = self.paused.lock().unwrap();
        *guard = paused;
        self.resumed.notify_all();
    }

    pub fn is_paused(&self) -> bool {
        *self.paused.lock().unwrap()
    }

    /// Called before each unit of work: waits while paused. `false` means stop.
    pub fn checkpoint(&self) -> bool {
        let mut paused = self.paused.lock().unwrap();
        loop {
            if self.stop_requested() {
                return false;
            }
            if !*paused {
                return true;
            }
            paused = self.resumed.wait(paused).unwrap();
        }
    }

    pub fn begin(&self, phase: Phase, total: u64) {
        let now = Instant::now();
        *self.last_event.lock().unwrap() = now;
        self.progress.set(Progress { phase, done: 0, total, failed: 0, bytes: 0, started: now });
    }

    /// One file done.
    pub fn add(&self, bytes: usize, failed: bool) {
        self.update(|p| {
            p.done += 1;
            p.failed += failed as u64;
            p.bytes += bytes as u64;
        });
    }

    /// Set the count directly (the scanner counts images found).
    pub fn set_done(&self, done: u64) {
        self.update(|p| p.done = done);
    }

    fn update(&self, change: impl FnOnce(&mut Progress)) {
        let mut last = self.last_event.lock().unwrap();
        let due = self.progress.update(|p| {
            change(p);
            (last.elapsed() >= EVENT_INTERVAL).then(|| p.clone())
        });
        if let Some(progress) = due {
            *last = Instant::now();
            self.events.publish(Event::Progress(progress));
        }
    }

    /// The phase ended: its final progress goes out as an event, and the phase is idle.
    pub fn finish(&self) {
        let ended = self.progress.update(|p| std::mem::replace(p, Progress::idle()));
        self.events.publish(Event::Finished(ended));
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
    fn finish_publishes_the_last_progress_and_goes_idle() {
        let ctl = Control::new();
        let sub = ctl.subscribe();
        ctl.begin(Phase::Import, 10);
        ctl.add(5, false);
        ctl.add(7, true);
        ctl.finish();
        let items = sub.take().items;
        match items.as_slice() {
            [Event::Finished(p)] => {
                assert_eq!((p.phase.clone(), p.done, p.failed, p.bytes), (Phase::Import, 2, 1, 12));
            }
            other => panic!("expected one Finished event, got {} events", other.len()),
        }
        assert_eq!(ctl.snapshot().phase, Phase::Idle);
    }
}
