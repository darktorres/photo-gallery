//! The per-server limit on the result files: at most [`PER_SERVER`] requests in flight to one server, counted across
//! every search, so a stopped search's downloads still count until they end. A download takes a [`Permit`] for each
//! server it is at, and the permit gives the slot back when it is dropped. A [`Kick`] is what the app's schedulers wait
//! on: a release wakes every subscribed kick, so a scheduler that was refused a slot tries again.
//!
//! Lock order, everywhere: a caller's own lock, then the gate's, then a kick's. A permit may be dropped under the
//! caller's lock, because dropping one takes only the gate's and the kicks'.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError, Weak};
use std::time::{Duration, Instant};

use crate::result_url::Server;

/// How many requests are in flight to one server at once.
pub const PER_SERVER: usize = 6;

/// A wait for a slot that the close cut short.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cancelled;

#[derive(Default)]
struct State {
    /// The requests in flight, per server.
    counts: BTreeMap<Server, usize>,
    /// The requests waiting for a slot, per server.
    waiters: BTreeMap<Server, usize>,
    /// The kicks to wake when a slot frees up.
    wakers: Vec<Weak<Kick>>,
}

/// The slots of every server: a count per server, shared by everything that fetches from one.
pub struct Gate {
    limit: usize,
    state: Mutex<State>,
    /// Signalled when a slot frees, and when the close is announced ([`Gate::wake`]).
    freed: Condvar,
}

fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    // A thread that panicked while holding it must not stop the others, so a poisoned lock is still used.
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Gives back one slot of `server`, and forgets the server once it has none in flight.
fn release_slot(counts: &mut BTreeMap<Server, usize>, server: &Server) {
    if let Some(n) = counts.get_mut(server) {
        *n = n.saturating_sub(1);
        if *n == 0 {
            counts.remove(server);
        }
    }
}

impl Gate {
    /// A gate that lets `limit` requests be in flight to each server.
    pub fn new(limit: usize) -> Arc<Gate> {
        Arc::new(Gate { limit, state: Mutex::default(), freed: Condvar::new() })
    }

    /// The requests in flight to `server`.
    pub fn in_flight(&self, server: &Server) -> usize {
        lock(&self.state).counts.get(server).copied().unwrap_or(0)
    }

    /// Whether `server` has a free slot, in `s`, the state the caller holds.
    fn has_room(&self, s: &State, server: &Server) -> bool {
        s.counts.get(server).copied().unwrap_or(0) < self.limit
    }

    /// Takes a slot of `server`, which the caller has checked is free, and returns its permit.
    fn take(self: &Arc<Self>, s: &mut State, server: &Server) -> Permit {
        *s.counts.entry(server.clone()).or_insert(0) += 1;
        Permit { gate: Arc::clone(self), server: server.clone() }
    }

    /// A slot of `server` if one is free and no request is waiting for one. Never waits.
    pub fn try_acquire(self: &Arc<Self>, server: &Server) -> Option<Permit> {
        let mut s = lock(&self.state);
        // A waiter has already done its part (it gave up its last slot to be queued again), so the new request must not
        // take the slot ahead of it.
        let waited_for = s.waiters.get(server).copied().unwrap_or(0) > 0;
        if waited_for || !self.has_room(&s, server) {
            return None;
        }
        Some(self.take(&mut s, server))
    }

    /// A slot of `server`, waited for while none is free. `Err(Cancelled)` when `cancel` is set first: it is checked
    /// under the lock before each wait, and [`Gate::wake`] is how a waiter that is already asleep learns of it.
    pub fn acquire(self: &Arc<Self>, server: &Server, cancel: &AtomicBool) -> Result<Permit, Cancelled> {
        let waiting = Waiting::enter(self, server);
        let mut s = lock(&self.state);
        let room = loop {
            if cancel.load(Ordering::Relaxed) {
                break Err(Cancelled);
            }
            if self.has_room(&s, server) {
                break Ok(());
            }
            s = self.freed.wait(s).unwrap_or_else(PoisonError::into_inner);
        };
        let permit = room.map(|()| self.take(&mut s, server));
        drop(s);
        // Leaving the queue may leave a free slot that no release will announce, so the kicks are told (see `Waiting`).
        drop(waiting);
        permit
    }

    /// Wakes every waiter in [`Gate::acquire`] so that each checks its cancel flag. Takes the lock first, so a waiter
    /// that has just checked the flag and not yet slept cannot miss it.
    pub fn wake(&self) {
        let _s = lock(&self.state);
        self.freed.notify_all();
    }

    /// Registers a kick to be woken when a slot frees up. A kick that has been dropped is forgotten.
    pub fn subscribe(&self, kick: Weak<Kick>) {
        let mut s = lock(&self.state);
        s.wakers.retain(|w| w.strong_count() > 0);
        s.wakers.push(kick);
    }

    /// Wakes every live kick. Called after the gate's lock is released, so a kick never waits on a slot.
    fn kick_wakers(&self) {
        let live: Vec<Arc<Kick>> = {
            let mut s = lock(&self.state);
            s.wakers.retain(|w| w.strong_count() > 0);
            s.wakers.iter().filter_map(Weak::upgrade).collect()
        };
        for kick in live {
            kick.kick();
        }
    }
}

/// A request waiting in [`Gate::acquire`]: counted while it waits, so [`Gate::try_acquire`] refuses to jump it. When it
/// leaves, with a slot free and no other waiter, the kicks are told: the schedulers may have been refused while it waited,
/// and no release will come to tell them.
struct Waiting<'a> {
    gate: &'a Gate,
    server: &'a Server,
}

impl<'a> Waiting<'a> {
    fn enter(gate: &'a Gate, server: &'a Server) -> Waiting<'a> {
        *lock(&gate.state).waiters.entry(server.clone()).or_insert(0) += 1;
        Waiting { gate, server }
    }
}

impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        let free_and_unwaited = {
            let mut s = lock(&self.gate.state);
            if let Some(n) = s.waiters.get_mut(self.server) {
                *n = n.saturating_sub(1);
                if *n == 0 {
                    s.waiters.remove(self.server);
                }
            }
            !s.waiters.contains_key(self.server) && self.gate.has_room(&s, self.server)
        };
        if free_and_unwaited {
            self.gate.kick_wakers();
        }
    }
}

/// A slot of one server, held while a request is in flight to it. Dropping it gives the slot back and wakes the kicks.
pub struct Permit {
    gate: Arc<Gate>,
    server: Server,
}

impl Permit {
    /// The server the slot is of.
    pub fn server(&self) -> &Server {
        &self.server
    }

    /// The gate the slot is of.
    pub fn gate(&self) -> &Arc<Gate> {
        &self.gate
    }
}

impl Drop for Permit {
    fn drop(&mut self) {
        {
            let mut s = lock(&self.gate.state);
            release_slot(&mut s.counts, &self.server);
        }
        self.gate.freed.notify_all();
        self.gate.kick_wakers();
    }
}

/// A wake-up for a scheduler: [`Kick::wait`] returns once it has been kicked, or closed.
#[derive(Default)]
pub struct Kick {
    state: Mutex<KickState>,
    cv: Condvar,
}

#[derive(Default)]
struct KickState {
    /// Kicked since the last `wait`.
    flag: bool,
    /// Closed for good: every wait and pause returns.
    closed: bool,
}

fn lock_kick(state: &Mutex<KickState>) -> MutexGuard<'_, KickState> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Kick {
    /// A kick, shared between the scheduler that waits on it and the gate that wakes it.
    pub fn new() -> Arc<Kick> {
        Arc::new(Kick::default())
    }

    /// Wakes the waiter. Touches nothing else, so it is safe to call from a drop under any lock.
    pub fn kick(&self) {
        lock_kick(&self.state).flag = true;
        self.cv.notify_all();
    }

    /// Waits until kicked (clearing the kick) or closed.
    pub fn wait(&self) {
        let mut s = lock_kick(&self.state);
        while !s.flag && !s.closed {
            s = self.cv.wait(s).unwrap_or_else(PoisonError::into_inner);
        }
        s.flag = false;
    }

    /// Sleeps for `d`, or until closed. A kick does not end it: a backoff must not be undone by the release that caused it.
    pub fn pause(&self, d: Duration) {
        let deadline = Instant::now() + d;
        let mut s = lock_kick(&self.state);
        while !s.closed {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            s = self.cv.wait_timeout(s, left).unwrap_or_else(PoisonError::into_inner).0;
        }
    }

    /// Closes the kick for good: every wait and pause returns from now on.
    pub fn close(&self) {
        lock_kick(&self.state).closed = true;
        self.cv.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use super::*;

    const WAIT: Duration = Duration::from_secs(10);

    fn server(port: u16) -> Server {
        crate::ResultUrl::parse(&format!("http://a.example:{port}/x")).unwrap().server().clone()
    }

    #[test]
    fn a_server_has_at_most_its_limit_in_flight() {
        let gate = Gate::new(2);
        let a = server(1);
        let first = gate.try_acquire(&a).unwrap();
        let _second = gate.try_acquire(&a).unwrap();
        assert!(gate.try_acquire(&a).is_none(), "the limit holds");
        assert!(gate.try_acquire(&server(2)).is_some(), "another server has its own slots");
        drop(first);
        assert!(gate.try_acquire(&a).is_some(), "a released slot is free again");
        assert_eq!(gate.in_flight(&a), 1, "the probe's permit was dropped; only the second slot is held");
    }

    #[test]
    fn a_blocked_acquire_gets_the_slot_when_it_is_released() {
        let gate = Gate::new(1);
        let a = server(1);
        let held = gate.try_acquire(&a).unwrap();
        let (tx, rx) = mpsc::channel();
        let waiter_gate = gate.clone();
        let cancel = AtomicBool::new(false);
        std::thread::spawn(move || {
            let got = waiter_gate.acquire(&server(1), &cancel).is_ok();
            let _ = tx.send(got);
        });
        assert!(rx.recv_timeout(Duration::from_millis(100)).is_err(), "still waiting while the slot is held");
        drop(held);
        assert_eq!(rx.recv_timeout(WAIT), Ok(true));
    }

    #[test]
    fn a_waiter_cancelled_by_the_close_ends_with_cancelled() {
        let gate = Gate::new(1);
        let a = server(1);
        let _held = gate.try_acquire(&a).unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let (waiter_gate, waiter_cancel) = (gate.clone(), cancel.clone());
        std::thread::spawn(move || {
            let _ = tx.send(waiter_gate.acquire(&server(1), &waiter_cancel).err());
        });
        std::thread::sleep(Duration::from_millis(50));
        cancel.store(true, Ordering::Relaxed);
        gate.wake();
        assert_eq!(rx.recv_timeout(WAIT), Ok(Some(Cancelled)));
    }

    #[test]
    fn a_release_wakes_the_subscribed_kicks_and_forgets_the_dropped_ones() {
        let gate = Gate::new(1);
        let kick = Kick::new();
        gate.subscribe(Arc::downgrade(&kick));
        {
            let dropped = Kick::new();
            gate.subscribe(Arc::downgrade(&dropped));
        }
        let permit = gate.try_acquire(&server(1)).unwrap();
        let (tx, rx) = mpsc::channel();
        let waiting = kick.clone();
        std::thread::spawn(move || {
            waiting.wait();
            let _ = tx.send(());
        });
        drop(permit);
        assert_eq!(rx.recv_timeout(WAIT), Ok(()));
    }

    #[test]
    fn a_kick_wakes_a_wait_once_and_a_close_ends_every_wait() {
        let kick = Kick::new();
        kick.kick();
        kick.wait();
        let (tx, rx) = mpsc::channel();
        let closing = kick.clone();
        std::thread::spawn(move || {
            closing.wait();
            let _ = tx.send(());
        });
        assert!(rx.recv_timeout(Duration::from_millis(100)).is_err(), "the kick was consumed by the first wait");
        kick.close();
        assert_eq!(rx.recv_timeout(WAIT), Ok(()));
    }

    /// A request that is waiting keeps its place: a new request for its server is refused while it waits, even with a slot
    /// free, and is admitted once it has gone.
    #[test]
    fn a_waiting_request_keeps_its_place_against_a_new_one() {
        let gate = Gate::new(1);
        let a = server(1);
        let waiting = Waiting::enter(&gate, &a);
        assert!(gate.try_acquire(&a).is_none(), "the waiter's place is kept");
        drop(waiting);
        assert!(gate.try_acquire(&a).is_some(), "with the waiter gone, the free slot is taken");
    }

    /// A waiter that leaves while a slot is free wakes the kicks: no release is coming to tell the schedulers.
    #[test]
    fn a_waiter_that_leaves_with_a_free_slot_kicks_the_schedulers() {
        let gate = Gate::new(1);
        let kick = Kick::new();
        gate.subscribe(Arc::downgrade(&kick));
        let (tx, rx) = mpsc::channel();
        let waiting = kick.clone();
        std::thread::spawn(move || {
            waiting.wait();
            let _ = tx.send(());
        });
        drop(Waiting::enter(&gate, &server(1)));
        assert_eq!(rx.recv_timeout(WAIT), Ok(()));
    }

    #[test]
    fn a_pause_is_not_ended_by_a_kick_but_is_by_a_close() {
        let kick = Kick::new();
        kick.kick();
        let started = Instant::now();
        kick.pause(Duration::from_millis(60));
        assert!(started.elapsed() >= Duration::from_millis(60), "the kick did not end the pause");
        kick.close();
        let started = Instant::now();
        kick.pause(Duration::from_secs(60));
        assert!(started.elapsed() < Duration::from_secs(5), "a close ends the pause");
    }
}
