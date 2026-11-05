//! How a running job tells others what changed. A [`Watch`] holds the latest value (the progress, the write status):
//! whoever looks gets the newest, and nothing queues up. A [`Bus`] carries each event to every subscriber (a pass's
//! progress, a round that ended). Each subscriber has a bounded queue: one that falls behind loses its oldest events,
//! and is told how many ([`Taken::lagged`]), so the newest always arrive.

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex, Weak};

/// The latest value of something, read by whoever asks.
pub struct Watch<T> {
    value: Mutex<T>,
}

impl<T: Clone> Watch<T> {
    pub fn new(value: T) -> Watch<T> {
        Watch { value: Mutex::new(value) }
    }

    pub fn get(&self) -> T {
        self.value.lock().unwrap().clone()
    }

    pub fn set(&self, value: T) {
        *self.value.lock().unwrap() = value;
    }

    /// Change the value in place (no copy: the progress changes once per file), and return what `change` returns.
    pub fn update<R>(&self, change: impl FnOnce(&mut T) -> R) -> R {
        change(&mut self.value.lock().unwrap())
    }
}

/// The events of one kind, to every subscriber, each with a queue of at most `cap` events.
pub struct Bus<E> {
    cap: usize,
    subscribers: Mutex<Vec<Weak<Inbox<E>>>>,
}

struct Inbox<E> {
    queue: Mutex<Queue<E>>,
    ready: Condvar,
}

struct Queue<E> {
    items: VecDeque<E>,
    /// Events dropped since the subscriber last took its events.
    lagged: usize,
    /// The bus is gone: nothing more comes.
    closed: bool,
}

/// What a subscriber took: its events, oldest first, and how many were dropped before them.
pub struct Taken<E> {
    pub items: Vec<E>,
    pub lagged: usize,
}

/// A subscriber's end of a bus. Dropping it unsubscribes it.
pub struct Subscription<E> {
    inbox: Arc<Inbox<E>>,
}

impl<E: Clone> Bus<E> {
    pub fn new(cap: usize) -> Bus<E> {
        Bus { cap: cap.max(1), subscribers: Mutex::new(Vec::new()) }
    }

    pub fn subscribe(&self) -> Subscription<E> {
        let inbox = Arc::new(Inbox {
            queue: Mutex::new(Queue { items: VecDeque::new(), lagged: 0, closed: false }),
            ready: Condvar::new(),
        });
        self.subscribers.lock().unwrap().push(Arc::downgrade(&inbox));
        Subscription { inbox }
    }

    pub fn publish(&self, event: E) {
        let mut subscribers = self.subscribers.lock().unwrap();
        subscribers.retain(|s| s.strong_count() > 0);
        for inbox in subscribers.iter().filter_map(Weak::upgrade) {
            let mut queue = inbox.queue.lock().unwrap();
            if queue.items.len() == self.cap {
                queue.items.pop_front();
                queue.lagged += 1;
            }
            queue.items.push_back(event.clone());
            inbox.ready.notify_one();
        }
    }
}

impl<E> Drop for Bus<E> {
    fn drop(&mut self) {
        for inbox in self.subscribers.lock().unwrap().iter().filter_map(Weak::upgrade) {
            inbox.queue.lock().unwrap().closed = true;
            inbox.ready.notify_all();
        }
    }
}

impl<E> Subscription<E> {
    /// Everything queued since the last take, and how many events were dropped before it.
    pub fn take(&self) -> Taken<E> {
        let mut queue = self.inbox.queue.lock().unwrap();
        Taken { items: queue.items.drain(..).collect(), lagged: std::mem::take(&mut queue.lagged) }
    }

    /// The next event, waiting for it. `None` once the bus is gone and everything it sent was taken.
    pub fn recv(&self) -> Option<E> {
        let mut queue = self.inbox.queue.lock().unwrap();
        loop {
            if let Some(event) = queue.items.pop_front() {
                return Some(event);
            }
            if queue.closed {
                return None;
            }
            queue = self.inbox.ready.wait(queue).unwrap();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_watch_holds_the_latest_value() {
        let watch = Watch::new(1u64);
        watch.set(2);
        watch.update(|v| *v += 3);
        assert_eq!(watch.get(), 5);
    }

    #[test]
    fn every_subscriber_gets_every_event_and_a_full_queue_counts_what_it_drops() {
        let bus = Bus::new(3);
        let first = bus.subscribe();
        let second = bus.subscribe();
        for i in 0..5 {
            bus.publish(i);
        }
        let taken = first.take();
        assert_eq!((taken.items, taken.lagged), (vec![2, 3, 4], 2));
        // Taken: the count starts again from zero.
        let taken = first.take();
        assert_eq!((taken.items.len(), taken.lagged), (0, 0));
        assert_eq!(second.take().items, vec![2, 3, 4]);
    }

    #[test]
    fn a_dropped_subscriber_is_forgotten() {
        let bus = Bus::new(4);
        let kept = bus.subscribe();
        drop(bus.subscribe());
        bus.publish("a");
        assert_eq!(kept.take().items, vec!["a"]);
        assert_eq!(bus.subscribers.lock().unwrap().len(), 1);
    }

    #[test]
    fn recv_gives_what_was_sent_then_none_once_the_bus_is_gone() {
        let bus = Bus::new(4);
        let sub = bus.subscribe();
        let sender = std::thread::spawn(move || {
            for i in 0..3 {
                bus.publish(i);
            }
            // Dropping the bus here closes the subscription after the three events.
        });
        let mut got = Vec::new();
        while let Some(event) = sub.recv() {
            got.push(event);
        }
        sender.join().unwrap();
        assert_eq!(got, vec![0, 1, 2]);
    }
}

/// What the library's background job or its writer changed, for an owner that reads the new state on its own thread
/// instead of polling. An event says only that something changed: the state is read from the job's and the writer's
/// getters. (The library's passes publish these; the enum lives here so the writer and the job share it without the
/// writer depending on the job.)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    /// The pass's progress, or the pause.
    Progress,
    /// A notice for the user (`Runner::take_messages`).
    Notice,
    /// A folder scan changed the catalog (`Runner::generation`).
    Scanned,
    /// An import or border pass analyzed something (`Runner::generation`).
    Imported,
    /// A round ended (`Runner::rounds`).
    Round,
    /// The writer's queue or its latest failure (`Writer::status`).
    Writes,
    /// A file was turned (`Writer::turned`).
    Turned,
}
