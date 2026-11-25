//! What a run's lanes share: the CPU, as permits every lane's workers take while they process a file, so two lanes' decode
//! pools share the cores instead of oversubscribing them. Memory is not shared out: a pass holds what its files in flight
//! take (no cap, by decision; `pipeline`).
//!
//! Permits are granted in arrival order. A worker that finishes gives its permit back and the next arrival takes it, so no
//! lane starves another; priorities alone don't make two pools fair.

use std::sync::{Arc, Condvar, Mutex, PoisonError};

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
}
