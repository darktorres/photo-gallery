//! The change pump: a thread that takes a bus's changes and queues a run of a QObject's handler onto the Qt thread for
//! them. What came in is gathered while a run is queued, so at most one is queued at a time, and the Qt thread runs one
//! per event-loop turn at most. The library's model and the Yandex panel each start one.

use core::pin::Pin;
use std::io;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use cxx_qt::{CxxQtThread, Threading};

/// The changes the pump has taken and the Qt thread hasn't handled (gathered into `P`), and whether a run is queued
/// for them.
#[derive(Default)]
pub(crate) struct Pending<P> {
    changes: P,
    queued: bool,
}

impl<P: Default> Pending<P> {
    /// What came in since the last run, taken on the Qt thread by the run: the next change queues another.
    pub(crate) fn take(shared: &Mutex<Pending<P>>) -> P {
        let mut pending = lock(shared);
        pending.queued = false;
        std::mem::take(&mut pending.changes)
    }
}

fn lock<P>(shared: &Mutex<Pending<P>>) -> MutexGuard<'_, Pending<P>> {
    // A panic on the other side leaves the changes as they were: the next run handles them.
    shared.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Starts the thread `name`: it waits for the next change (`recv`; `None` when the bus is gone, which ends it), gathers
/// it (`add`), and queues `run` onto `thread`'s object when no run is queued. It ends with the window too (the queue
/// fails then). An error when the thread couldn't start.
pub(crate) fn start<T, C, P>(
    name: &str,
    thread: CxxQtThread<T>,
    mut recv: impl FnMut() -> Option<C> + Send + 'static,
    add: impl Fn(&mut P, C) + Send + 'static,
    run: fn(Pin<&mut T>, Arc<Mutex<Pending<P>>>),
) -> io::Result<()>
where
    T: Threading + 'static,
    P: Default + Send + 'static,
{
    let shared: Arc<Mutex<Pending<P>>> = Arc::default();
    std::thread::Builder::new()
        .name(name.into())
        .spawn(move || {
            while let Some(change) = recv() {
                // Queued only when no run is: a run that is queued reads what this change is added to.
                let queue = {
                    let mut pending = lock(&shared);
                    add(&mut pending.changes, change);
                    !std::mem::replace(&mut pending.queued, true)
                };
                if queue {
                    let pending = Arc::clone(&shared);
                    if thread.queue(move |object| run(object, pending)).is_err() {
                        break;
                    }
                }
            }
        })
        .map(drop)
}
