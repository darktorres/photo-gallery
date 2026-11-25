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
/// held come back; it never frees a buffer in use. It also counts the bytes its pools hold now, which the governor's soft
/// cap reads ([`soft_cap`]).
#[derive(Debug)]
pub struct Budget {
    bytes: AtomicU64,
    held: AtomicU64,
}

impl Budget {
    /// A budget of `bytes` (at least one byte).
    pub fn new(bytes: u64) -> Budget {
        Budget { bytes: AtomicU64::new(bytes.max(1)), held: AtomicU64::new(0) }
    }

    /// The budget now.
    pub fn get(&self) -> u64 {
        self.bytes.load(Ordering::SeqCst)
    }

    /// Sets the budget (at least one byte).
    pub fn set(&self, bytes: u64) {
        self.bytes.store(bytes.max(1), Ordering::SeqCst);
    }

    /// The bytes its pools hold now (lent out, or idle in a pool).
    pub fn held(&self) -> u64 {
        self.held.load(Ordering::SeqCst)
    }

    /// A pool holds `bytes` more.
    pub fn hold(&self, bytes: u64) {
        self.move_held(|held| held.saturating_add(bytes));
    }

    /// A pool holds `bytes` less.
    pub fn release(&self, bytes: u64) {
        self.move_held(|held| held.saturating_sub(bytes));
    }

    /// Sets what is held to `to(held)`, atomically.
    fn move_held(&self, to: impl Fn(u64) -> u64) {
        let mut held = self.held.load(Ordering::SeqCst);
        loop {
            match self.held.compare_exchange_weak(held, to(held), Ordering::SeqCst, Ordering::SeqCst) {
                Ok(_) => return,
                Err(now) => held = now,
            }
        }
    }
}

/// No cap, as the governor's atomic holds it.
const NO_CAP: u64 = u64::MAX;

/// The file-buffer budgets of a run's lanes. Each lane holds one [`Budget`], which the governor sets: the lane's
/// `knee` (what one lane wants, the measured one: `pg_jobs::Options::budget`), or, when `cap` (the most every lane
/// holds together) binds, the cap split evenly among the lanes. A lane that is retired drops its budget, and the next
/// [`Governor::rebalance`] gives its share to the others. The cap changes as the machine's memory does
/// ([`Governor::set_cap`], from [`soft_cap`]): the coordinator sets it on a timer and rebalances when it moved.
pub struct Governor {
    knee: u64,
    cap: AtomicU64,
    lanes: Mutex<Vec<Weak<Budget>>>,
}

/// A cap's relative change under which [`Governor::set_cap`] keeps the old one: small moves of the machine's free memory
/// rebalance nothing.
pub const CAP_HYSTERESIS: f64 = 0.05;

impl Governor {
    /// A governor of lanes that each want `knee` bytes, and together hold at most `cap` (`None`: no cap).
    pub fn new(knee: u64, cap: Option<u64>) -> Governor {
        Governor { knee: knee.max(1), cap: AtomicU64::new(cap.unwrap_or(NO_CAP)), lanes: Mutex::new(Vec::new()) }
    }

    /// The cap now (`None`: no cap).
    pub fn cap(&self) -> Option<u64> {
        let cap = self.cap.load(Ordering::SeqCst);
        (cap != NO_CAP).then_some(cap)
    }

    /// Sets the cap, unless it moved by less than [`CAP_HYSTERESIS`] of the old one. Whether it changed: the caller then
    /// rebalances.
    pub fn set_cap(&self, cap: Option<u64>) -> bool {
        let old = self.cap();
        if !moved(old, cap) {
            return false;
        }
        self.cap.store(cap.unwrap_or(NO_CAP), Ordering::SeqCst);
        true
    }

    /// The bytes every lane's pools hold now.
    pub fn held(&self) -> u64 {
        let lanes = self.lanes.lock().unwrap_or_else(PoisonError::into_inner);
        lanes.iter().filter_map(Weak::upgrade).map(|budget| budget.held()).fold(0, u64::saturating_add)
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
        let share = split(self.knee, self.cap(), lanes.len());
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

/// Whether a cap moved from `old` to `new` by enough to change it ([`CAP_HYSTERESIS`]): a cap that appears or goes always
/// does.
fn moved(old: Option<u64>, new: Option<u64>) -> bool {
    match (old, new) {
        (None, None) => false,
        (Some(old), Some(new)) => {
            let (old, new) = (pg_model::num::size(old), pg_model::num::size(new));
            (new - old).abs() > old * CAP_HYSTERESIS
        }
        _ => true,
    }
}

/// The run's soft cap on what its lanes' buffers hold together: the lesser of `physical / share` (a fixed share of the
/// machine) and `held + available − headroom` (what the lanes hold now, and what the machine has free beyond the
/// headroom). `held` is added back so the cap does not ratchet: as the lanes fill, `available` falls by what they took,
/// and the sum stays. `None` when the system says neither.
pub fn soft_cap(physical: Option<u64>, available: Option<u64>, held: u64, share: u64, headroom: u64) -> Option<u64> {
    let fixed = physical.map(|bytes| bytes / share.max(1));
    let live = available.map(|bytes| held.saturating_add(bytes).saturating_sub(headroom));
    match (fixed, live) {
        (Some(fixed), Some(live)) => Some(fixed.min(live)),
        (one, None) | (None, one) => one,
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

    // As the lanes fill, the machine's free memory falls by what they took: the cap stays. As they empty, it is back.
    #[test]
    fn the_soft_cap_does_not_ratchet_as_the_lanes_fill_and_empty() {
        let (physical, headroom) = (Some(64 << 30), 2 << 30);
        let free = 10u64 << 30;
        let empty = soft_cap(physical, Some(free), 0, 4, headroom);
        assert_eq!(empty, Some(8 << 30), "the live bound (10 − 2 GiB) is under a quarter (16 GiB)");
        for taken in [1u64 << 30, 4 << 30, 8 << 30] {
            assert_eq!(soft_cap(physical, Some(free - taken), taken, 4, headroom), empty, "{taken} held");
        }
        // Something else takes memory: the cap falls.
        assert_eq!(soft_cap(physical, Some(4 << 30), 0, 4, headroom), Some(2 << 30));
    }

    #[test]
    fn the_soft_cap_is_the_fixed_share_where_memory_is_free_and_none_without_facts() {
        assert_eq!(soft_cap(Some(16 << 30), Some(15 << 30), 0, 4, 1 << 30), Some(4 << 30));
        assert_eq!(soft_cap(Some(16 << 30), None, 0, 4, 1 << 30), Some(4 << 30));
        assert_eq!(soft_cap(None, Some(3 << 30), 0, 4, 1 << 30), Some(2 << 30));
        assert_eq!(soft_cap(None, None, 0, 4, 1 << 30), None);
        assert_eq!(soft_cap(None, Some(1 << 20), 0, 4, 1 << 30), Some(0), "no room beyond the headroom");
    }

    #[test]
    fn a_small_move_of_the_cap_changes_nothing_and_a_large_one_rebalances() {
        let governor = Governor::new(240, Some(1000));
        let lane = governor.lane();
        assert!(!governor.set_cap(Some(1040)), "4%: kept");
        assert_eq!(governor.cap(), Some(1000));
        assert!(governor.set_cap(Some(300)));
        governor.rebalance();
        assert_eq!(lane.get(), 240, "one lane: its knee, under the cap");
        let second = governor.lane();
        assert_eq!((lane.get(), second.get()), (150, 150), "the new cap is split");
        assert!(governor.set_cap(None), "no cap any more");
        governor.rebalance();
        assert_eq!((lane.get(), second.get()), (240, 240));
    }

    #[test]
    fn a_budget_counts_what_its_pools_hold() {
        let governor = Governor::new(240, None);
        let (a, b) = (governor.lane(), governor.lane());
        a.hold(100);
        b.hold(50);
        a.release(30);
        assert_eq!((a.held(), b.held(), governor.held()), (70, 50, 120));
        b.release(500);
        assert_eq!(b.held(), 0, "never below zero");
    }

    #[test]
    fn a_budget_set_takes_effect_for_the_next_read() {
        let budget = Budget::new(0);
        assert_eq!(budget.get(), 1, "at least one byte");
        budget.set(4096);
        assert_eq!(budget.get(), 4096);
    }
}
