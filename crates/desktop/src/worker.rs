//! The window's writes, off its thread. A rating, a turn or a border mark waits for the catalog's write lock (an import
//! or the writer holds it), and the preferences and the session are written to disk: the Qt thread waits for neither.
//! One thread carries the writes out, in the order they were asked for, so two writes to one file land in that order.
//!
//! The thread is started by `start` and ends at `stop`, after the writes asked for so far. A write asked for with no
//! thread (before `start`, or after `stop`) is carried out where it is asked for: none is dropped.

use std::sync::mpsc::{self, Sender};
use std::sync::{Mutex, OnceLock, PoisonError};
use std::thread::JoinHandle;

/// A write to carry out.
type Write = Box<dyn FnOnce() + Send>;

/// The thread's queue and its handle: `stop` takes both.
struct Worker {
    queue: Mutex<Option<Sender<Write>>>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

static WORKER: OnceLock<Worker> = OnceLock::new();

/// Start the thread that carries the writes out. An error when it can't be started (the writes then run where they are
/// asked for) or when it was started before.
pub(crate) fn start() -> std::io::Result<()> {
    let (queue, writes) = mpsc::channel::<Write>();
    let thread = std::thread::Builder::new().name("window writes".into()).spawn(move || {
        for write in writes {
            write();
        }
    })?;
    let worker = Worker { queue: Mutex::new(Some(queue)), thread: Mutex::new(Some(thread)) };
    WORKER.set(worker).map_err(|_| std::io::Error::other("the window's writes were started twice"))
}

/// Carry out `write` on the thread, after the writes asked for before it. With no thread (not started, or stopped) it
/// is carried out here.
pub(crate) fn run(write: impl FnOnce() + Send + 'static) {
    let job: Write = Box::new(write);
    let unsent = match WORKER.get() {
        Some(worker) => match worker.queue.lock().unwrap_or_else(PoisonError::into_inner).as_ref() {
            Some(queue) => queue.send(job).map_err(|mpsc::SendError(job)| job),
            None => Err(job),
        },
        None => Err(job),
    };
    if let Err(job) = unsent {
        job();
    }
}

/// Stop the thread once the writes asked for so far are carried out: the queue closes, and the thread ends after them.
/// The window calls it before the catalog closes.
pub(crate) fn stop() {
    let Some(worker) = WORKER.get() else { return };
    worker.queue.lock().unwrap_or_else(PoisonError::into_inner).take();
    let thread = worker.thread.lock().unwrap_or_else(PoisonError::into_inner).take();
    if let Some(thread) = thread
        && thread.join().is_err()
    {
        pg_log::error("window", "the window's writes thread panicked");
    }
}
