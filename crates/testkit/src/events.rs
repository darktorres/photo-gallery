//! A bus that fails its test when the test ends, if anything was published on it. The code under test that reports
//! through a bus (the thumbnail cache's events) gets one from here: an event a test expects is read from its own
//! subscription, and any other fails the test, as the panicking report it replaces did.

use std::fmt::Debug;
use std::sync::Arc;

use pg_jobs::events::{Bus, Subscription};

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
