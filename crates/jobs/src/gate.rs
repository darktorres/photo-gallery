//! The reader gate of a pipeline: how many of its reader threads may claim items now. A pass starts its readers up to
//! their most (`Readers::max`), and the gate lets the first `target` of them through; the others park until the target
//! grows, the pass ends, or its items run out. The tuner (`crate::tuner`) moves the target as it learns the drive.
//!
//! A reader asks the gate before it claims an item, never after: the pool grants buffers strictly in item order, so a
//! reader that parked holding a claimed index would hold every later item back. A parked reader wakes at least every
//! [`PARK`], so it sees a stop, a pause's end or the items' end even without a notification.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex, PoisonError};
use std::time::Duration;

/// The longest a parked reader waits before it looks at its pass again.
pub(crate) const PARK: Duration = Duration::from_millis(100);

/// The gate the readers of one pass share.
pub(crate) struct ReaderGate {
    /// The readers let through: readers `0..target`.
    target: AtomicUsize,
    /// The pass has ended, or its items ran out: no reader waits any more.
    closed: AtomicBool,
    lock: Mutex<()>,
    changed: Condvar,
}

impl ReaderGate {
    /// A gate that lets `target` readers through.
    pub(crate) fn new(target: usize) -> ReaderGate {
        ReaderGate {
            target: AtomicUsize::new(target),
            closed: AtomicBool::new(false),
            lock: Mutex::new(()),
            changed: Condvar::new(),
        }
    }

    /// The readers let through now.
    pub(crate) fn target(&self) -> usize {
        self.target.load(Ordering::SeqCst)
    }

    /// Lets `target` readers through from now on: a reader past it parks before its next claim, one under it wakes.
    pub(crate) fn set_target(&self, target: usize) {
        let _guard = self.lock.lock().unwrap_or_else(PoisonError::into_inner);
        self.target.store(target, Ordering::SeqCst);
        self.changed.notify_all();
    }

    /// Ends every wait: the pass has ended (`halt`), or a reader found no item left.
    pub(crate) fn close(&self) {
        let _guard = self.lock.lock().unwrap_or_else(PoisonError::into_inner);
        self.closed.store(true, Ordering::SeqCst);
        self.changed.notify_all();
    }

    /// Whether reader `id` may go on to claim an item. It waits while it is past the target. `false` once the gate is
    /// closed, or `leave` says the reader is done (its pass stopped, or every item is claimed): it then ends.
    pub(crate) fn enter(&self, id: usize, leave: impl Fn() -> bool) -> bool {
        let mut guard = self.lock.lock().unwrap_or_else(PoisonError::into_inner);
        loop {
            if self.closed.load(Ordering::SeqCst) || leave() {
                return false;
            }
            if id < self.target.load(Ordering::SeqCst) {
                return true;
            }
            guard = self.changed.wait_timeout(guard, PARK).unwrap_or_else(PoisonError::into_inner).0;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::mpsc;

    use super::*;

    /// `gate.enter(id, leave)` on a thread of its own, its answer sent back.
    fn enter_on_thread(gate: &Arc<ReaderGate>, id: usize, leave: Arc<AtomicBool>) -> mpsc::Receiver<bool> {
        let (tx, rx) = mpsc::channel();
        let gate = Arc::clone(gate);
        std::thread::spawn(move || {
            let _ = tx.send(gate.enter(id, || leave.load(Ordering::SeqCst)));
        });
        rx
    }

    #[test]
    fn a_reader_under_the_target_goes_through_at_once() {
        let gate = ReaderGate::new(2);
        assert!(gate.enter(0, || false));
        assert!(gate.enter(1, || false));
    }

    #[test]
    fn a_reader_past_the_target_parks_until_the_target_grows() {
        let gate = Arc::new(ReaderGate::new(1));
        let parked = enter_on_thread(&gate, 2, Arc::new(AtomicBool::new(false)));
        assert!(parked.recv_timeout(PARK * 2).is_err(), "reader 2 parks while the target is 1");
        gate.set_target(3);
        assert_eq!(parked.recv_timeout(Duration::from_secs(5)), Ok(true), "a larger target lets it through");
    }

    #[test]
    fn closing_the_gate_releases_a_parked_reader() {
        let gate = Arc::new(ReaderGate::new(1));
        let parked = enter_on_thread(&gate, 1, Arc::new(AtomicBool::new(false)));
        assert!(parked.recv_timeout(PARK * 2).is_err());
        gate.close();
        assert_eq!(parked.recv_timeout(Duration::from_secs(5)), Ok(false), "closed: the reader ends");
        assert!(!gate.enter(0, || false), "a closed gate lets no reader through");
    }

    // A stop, or the items running out, reaches a parked reader within its park step, with no notification.
    #[test]
    fn a_parked_reader_leaves_when_its_pass_is_done_without_a_notification() {
        let gate = Arc::new(ReaderGate::new(0));
        let leave = Arc::new(AtomicBool::new(false));
        let parked = enter_on_thread(&gate, 0, leave.clone());
        assert!(parked.recv_timeout(PARK * 2).is_err());
        leave.store(true, Ordering::SeqCst);
        assert_eq!(parked.recv_timeout(PARK * 10), Ok(false));
    }

    #[test]
    fn a_smaller_target_holds_the_readers_past_it() {
        let gate = ReaderGate::new(4);
        gate.set_target(2);
        assert_eq!(gate.target(), 2);
        assert!(gate.enter(1, || false));
        let gate = Arc::new(gate);
        let parked = enter_on_thread(&gate, 3, Arc::new(AtomicBool::new(false)));
        assert!(parked.recv_timeout(PARK * 2).is_err(), "reader 3 is past the new target");
        gate.close();
        assert_eq!(parked.recv_timeout(Duration::from_secs(5)), Ok(false));
    }
}
