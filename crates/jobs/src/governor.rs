//! What a run's lanes share: the CPU, as permits every lane's workers take while they process a file, so two lanes' decode
//! pools share the cores instead of oversubscribing them; and the split of a memory budget among the lanes.
//!
//! Permits are granted in arrival order. A worker that finishes gives its permit back and the next arrival takes it, so no
//! lane starves another; priorities alone don't make two pools fair.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError, Weak};

/// The CPU permits the run's workers share.
pub struct CpuGate {
    state: Mutex<GateState>,
    changed: Condvar,
}

struct GateState {
    /// Permits not held.
    free: usize,
    /// The ticket the next arrival takes.
    next: u64,
    /// The ticket that may take the next permit.
    serving: u64,
}

/// A permit, given back when dropped.
pub struct Permit {
    gate: Arc<CpuGate>,
}

impl CpuGate {
    /// A gate of `permits` (at least one).
    pub fn new(permits: usize) -> CpuGate {
        CpuGate { state: Mutex::new(GateState { free: permits.max(1), next: 0, serving: 0 }), changed: Condvar::new() }
    }

    /// Waits for a permit, in arrival order.
    pub fn acquire(self: &Arc<Self>) -> Permit {
        let mut st = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let ticket = st.next;
        st.next += 1;
        while st.serving != ticket || st.free == 0 {
            st = self.changed.wait(st).unwrap_or_else(PoisonError::into_inner);
        }
        st.serving += 1;
        st.free -= 1;
        self.changed.notify_all();
        Permit { gate: Arc::clone(self) }
    }

    /// The permits not held now.
    pub fn free(&self) -> usize {
        self.state.lock().unwrap_or_else(PoisonError::into_inner).free
    }

    fn give_back(&self) {
        let mut st = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        st.free += 1;
        self.changed.notify_all();
    }
}

impl Drop for Permit {
    fn drop(&mut self) {
        self.gate.give_back();
    }
}

/// The bytes of file buffers a pass may hold, as the run's governor sets them: read by the pass's pool on every grant
/// (and every wait, so a change is seen within the pool's wait step). Shrinking it holds back grants until the buffers
/// held come back; it never frees a buffer in use.
#[derive(Debug)]
pub struct Budget(AtomicU64);

impl Budget {
    /// A budget of `bytes` (at least one byte).
    pub fn new(bytes: u64) -> Budget {
        Budget(AtomicU64::new(bytes.max(1)))
    }

    /// The budget now.
    pub fn get(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }

    /// Sets the budget (at least one byte).
    pub fn set(&self, bytes: u64) {
        self.0.store(bytes.max(1), Ordering::SeqCst);
    }
}

/// The file-buffer budgets of a run's lanes. Each lane holds one [`Budget`], which the governor sets: the lane's
/// `knee` (what one lane wants, the measured one: `pg_jobs::Options::budget`), or, when `cap` (the most every lane
/// holds together) binds, the cap split evenly among the lanes. A lane that is retired drops its budget, and the next
/// [`Governor::rebalance`] gives its share to the others.
pub struct Governor {
    knee: u64,
    cap: Option<u64>,
    lanes: Mutex<Vec<Weak<Budget>>>,
}

impl Governor {
    /// A governor of lanes that each want `knee` bytes, and together hold at most `cap` (`None`: no cap).
    pub fn new(knee: u64, cap: Option<u64>) -> Governor {
        Governor { knee: knee.max(1), cap, lanes: Mutex::new(Vec::new()) }
    }

    /// A new lane's budget, registered, with every lane's share rebalanced to include it.
    pub fn lane(&self) -> Arc<Budget> {
        let budget = Arc::new(Budget::new(self.knee));
        self.lanes.lock().unwrap_or_else(PoisonError::into_inner).push(Arc::downgrade(&budget));
        self.rebalance();
        budget
    }

    /// Sets every lane still held to its share of the cap: the knee, or the cap split among them when it binds. A
    /// retired lane (its budget dropped) is forgotten, and its share goes to the rest.
    pub fn rebalance(&self) {
        let mut lanes = self.lanes.lock().unwrap_or_else(PoisonError::into_inner);
        lanes.retain(|lane| lane.strong_count() > 0);
        let share = split(self.knee, self.cap, lanes.len());
        for budget in lanes.iter().filter_map(Weak::upgrade) {
            budget.set(share);
        }
    }

    /// The lanes held now.
    pub fn lanes(&self) -> usize {
        let mut lanes = self.lanes.lock().unwrap_or_else(PoisonError::into_inner);
        lanes.retain(|lane| lane.strong_count() > 0);
        lanes.len()
    }
}

/// One lane's budget when `lanes` lanes each want `knee` bytes and together may hold `cap`: the knee, unless the cap split
/// evenly among them is less.
pub fn split(knee: u64, cap: Option<u64>, lanes: usize) -> u64 {
    match cap {
        Some(cap) => knee.min(cap / u64::try_from(lanes.max(1)).unwrap_or(u64::MAX)).max(1),
        None => knee,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::time::Duration;

    use super::*;

    #[test]
    fn no_more_permits_are_held_than_the_gate_has() {
        let gate = Arc::new(CpuGate::new(2));
        let (held, most) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        let workers: Vec<_> = (0..8)
            .map(|_| {
                let (gate, held, most) = (gate.clone(), held.clone(), most.clone());
                std::thread::spawn(move || {
                    for _ in 0..20 {
                        let _permit = gate.acquire();
                        let now = held.fetch_add(1, Ordering::SeqCst) + 1;
                        most.fetch_max(now, Ordering::SeqCst);
                        std::thread::sleep(Duration::from_millis(1));
                        held.fetch_sub(1, Ordering::SeqCst);
                    }
                })
            })
            .collect();
        for worker in workers {
            worker.join().unwrap();
        }
        assert!(most.load(Ordering::SeqCst) <= 2, "{} held at once", most.load(Ordering::SeqCst));
        assert_eq!(gate.free(), 2, "every permit is given back");
    }

    #[test]
    fn waiters_take_the_permit_in_arrival_order() {
        let gate = Arc::new(CpuGate::new(1));
        let first = gate.acquire();
        let (tx, rx) = mpsc::channel();
        for name in ["a", "b"] {
            let (gate, tx) = (gate.clone(), tx.clone());
            std::thread::spawn(move || tx.send((name, gate.acquire())).unwrap());
            // Let each arrive before the next.
            std::thread::sleep(Duration::from_millis(50));
        }
        drop(first);
        let (name, permit) = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(name, "a", "the first to arrive is served first");
        drop(permit);
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap().0, "b");
    }

    #[test]
    fn without_a_cap_every_lane_keeps_its_knee() {
        assert_eq!(split(240, None, 3), 240);
        assert_eq!(split(240, None, 0), 240);
    }

    #[test]
    fn a_cap_that_binds_is_split_evenly_and_never_above_the_knee() {
        assert_eq!(split(240, Some(300), 2), 150, "the cap binds: half each");
        assert_eq!(split(240, Some(300), 1), 240, "a lone lane wants no more than its knee");
        assert_eq!(split(240, Some(0), 4), 1, "never zero, so a pool still grants one file");
    }

    #[test]
    fn lanes_come_and_go_and_the_rest_take_the_cap_between_them() {
        let governor = Governor::new(240, Some(300));
        let first = governor.lane();
        assert_eq!(first.get(), 240, "alone, it is under the cap");
        let second = governor.lane();
        assert_eq!((first.get(), second.get()), (150, 150), "two lanes split the cap");
        drop(second);
        governor.rebalance();
        assert_eq!(governor.lanes(), 1);
        assert_eq!(first.get(), 240, "the retired lane's share is the other's again");
    }

    #[test]
    fn a_budget_set_takes_effect_for_the_next_read() {
        let budget = Budget::new(0);
        assert_eq!(budget.get(), 1, "at least one byte");
        budget.set(4096);
        assert_eq!(budget.get(), 4096);
    }
}
