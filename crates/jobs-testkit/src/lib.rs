//! Test helpers over the job layer. A bus that fails its test when the test ends, if anything was published on it
//! ([`Quiet`]): the code under test that reports through a bus (the thumbnail cache's events) gets one from here, an
//! event a test expects is read from its own subscription, and any other fails the test, as the panicking report it
//! replaces did. And a notice sink that records what it is told ([`RecordingSink`]), for the tests of the workers that
//! post through one (the job, the writer).

use std::fmt::Debug;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use pg_jobs::NoticeSink;
use pg_jobs::events::{Bus, Subscription};
use pg_model::notice::{KeyBuf, Notice};

/// What a [`RecordingSink`] was told, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Call {
    Post(Notice),
    Replace(KeyBuf, Vec<Notice>),
    Clear(KeyBuf),
}

/// A notice sink that keeps every call, for a test to look at ([`RecordingSink::take`]), and counts the pokes.
#[derive(Default)]
pub struct RecordingSink {
    calls: Mutex<Vec<Call>>,
    pokes: AtomicUsize,
}

impl RecordingSink {
    pub fn new() -> Arc<RecordingSink> {
        Arc::new(RecordingSink::default())
    }

    /// The calls since the last take, in order.
    pub fn take(&self) -> Vec<Call> {
        std::mem::take(&mut *self.calls.lock().unwrap_or_else(PoisonError::into_inner))
    }

    /// The calls since the last take, left in place.
    pub fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// The notices posted since the last take, in order (the replaces and clears left out).
    pub fn posted(&self) -> Vec<Notice> {
        self.calls()
            .into_iter()
            .filter_map(|call| match call {
                Call::Post(notice) => Some(notice),
                Call::Replace(..) | Call::Clear(_) => None,
            })
            .collect()
    }

    /// The pokes so far.
    pub fn pokes(&self) -> usize {
        self.pokes.load(Ordering::SeqCst)
    }

    fn push(&self, call: Call) {
        self.calls.lock().unwrap_or_else(PoisonError::into_inner).push(call);
    }
}

impl NoticeSink for RecordingSink {
    fn post(&self, notice: Notice) {
        self.push(Call::Post(notice));
    }

    fn replace(&self, scope: KeyBuf, notices: Vec<Notice>) {
        self.push(Call::Replace(scope, notices));
    }

    fn clear(&self, key: KeyBuf) {
        self.push(Call::Clear(key));
    }

    fn poke(&self) {
        self.pokes.fetch_add(1, Ordering::SeqCst);
    }
}

/// The watch on a bus: dropped at the end of its test, it fails the test if anything was published and not taken.
pub struct Quiet<E: Clone + Debug> {
    events: Subscription<E>,
}

impl<E: Clone + Debug> Quiet<E> {
    /// A bus to give the code under test, and the watch on it. Declare the watch before what publishes on the bus, so
    /// the watch is dropped after it (locals drop in reverse order).
    pub fn bus() -> (Arc<Bus<E>>, Quiet<E>) {
        let bus = Arc::new(Bus::new(64));
        let events = bus.subscribe();
        (bus, Quiet { events })
    }
}

impl<E: Clone + Debug> Quiet<E> {
    /// Fails the test if anything was published and not taken. A test whose cache is leaked (never dropped) calls this
    /// last; the drop does it otherwise.
    pub fn check(&self) {
        let taken = self.events.take();
        assert!(taken.items.is_empty(), "events nobody expected: {:?}", taken.items);
    }
}

impl<E: Clone + Debug> Drop for Quiet<E> {
    fn drop(&mut self) {
        // A test that already failed keeps its own message.
        if !std::thread::panicking() {
            self.check();
        }
    }
}
