//! A thread that does work when it is woken: the app's background job, the file writer. The body runs on the thread
//! and asks its [`Waker`] for the next wake (work to do, or the idle time passed, or a quit). `wake` and `stop` are
//! called from elsewhere, and the thread ends when its body returns.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

struct Shared {
    /// Set by `wake`, cleared by the body's wait.
    woken: Mutex<bool>,
    ready: Condvar,
    quit: AtomicBool,
    /// Set when the body has returned (or panicked), and signalled, so a bounded stop can wait for it.
    exited: Mutex<bool>,
    exited_signal: Condvar,
}

/// Marks the thread as exited when the body's frame is left, by a return or by a panic.
struct Exit(Arc<Shared>);

impl Drop for Exit {
    fn drop(&mut self) {
        *self.0.exited.lock().unwrap_or_else(PoisonError::into_inner) = true;
        self.0.exited_signal.notify_all();
    }
}

/// Why the body's wait returned.
#[derive(Debug, PartialEq, Eq)]
pub enum Woke {
    /// Something was asked for since the last wait.
    Work,
    /// The idle time passed with nothing asked for (only with a timeout).
    Idle,
    /// The service is stopping: the body returns.
    Quit,
}

/// The body's side of the wake-up.
pub struct Waker<'a> {
    shared: &'a Shared,
}

impl Waker<'_> {
    /// Wait for work, the quit, or, with `idle`, that long. Work asked for before the wait returns at once.
    pub fn wait(&self, idle: Option<Duration>) -> Woke {
        let mut woken = self.shared.woken.lock().unwrap_or_else(PoisonError::into_inner);
        match idle {
            None => {
                while !*woken && !self.quitting() {
                    woken = self.shared.ready.wait(woken).unwrap_or_else(PoisonError::into_inner);
                }
            }
            Some(timeout) => {
                if !*woken && !self.quitting() {
                    woken = self.shared.ready.wait_timeout(woken, timeout).unwrap_or_else(PoisonError::into_inner).0;
                }
            }
        }
        if self.quitting() {
            return Woke::Quit;
        }
        if std::mem::take(&mut *woken) { Woke::Work } else { Woke::Idle }
    }

    /// Whether the service is stopping (checked between units of work, so a body can end early).
    pub fn quitting(&self) -> bool {
        self.shared.quit.load(Ordering::SeqCst)
    }
}

pub struct Service {
    shared: Arc<Shared>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl Service {
    /// Start the thread, running `body`. An error if the thread can't be spawned (the body is not run).
    pub fn start(name: &str, body: impl FnOnce(&Waker) + Send + 'static) -> std::io::Result<Service> {
        // Woken from the start: the body's first wait returns at once, so a new service does its first pass.
        let shared = Arc::new(Shared {
            woken: Mutex::new(true),
            ready: Condvar::new(),
            quit: AtomicBool::new(false),
            exited: Mutex::new(false),
            exited_signal: Condvar::new(),
        });
        let job = shared.clone();
        let thread = std::thread::Builder::new().name(name.into()).spawn(move || {
            let _exit = Exit(job.clone());
            body(&Waker { shared: &job });
        })?;
        Ok(Service { shared, thread: Mutex::new(Some(thread)) })
    }

    /// Ask the body for a wake: it returns from its wait with [`Woke::Work`].
    pub fn wake(&self) {
        *self.shared.woken.lock().unwrap_or_else(PoisonError::into_inner) = true;
        self.shared.ready.notify_one();
    }

    /// Ask the body to return at its next wait (or as soon as it checks [`Waker::quitting`]).
    pub fn quit(&self) {
        // Under the lock, so a body between its check and its wait can't miss it.
        let _woken = self.shared.woken.lock().unwrap_or_else(PoisonError::into_inner);
        self.shared.quit.store(true, Ordering::SeqCst);
        self.shared.ready.notify_all();
    }

    /// Quit, and wait for the thread to end, for at most `timeout`. `false` if it is still running then; it ends with
    /// the process. Idempotent.
    pub fn stop_within(&self, timeout: Duration) -> bool {
        self.quit();
        let exited = self.shared.exited.lock().unwrap_or_else(PoisonError::into_inner);
        let (exited, _) = self
            .shared
            .exited_signal
            .wait_timeout_while(exited, timeout, |done| !*done)
            .unwrap_or_else(PoisonError::into_inner);
        let finished = *exited;
        drop(exited);
        if finished && let Some(thread) = self.thread.lock().unwrap_or_else(PoisonError::into_inner).take() {
            let _ = thread.join();
        }
        finished
    }

    /// Quit, and wait for the thread to end. Idempotent; dropping the service does it too.
    pub fn stop(&self) {
        self.quit();
        if let Some(thread) = self.thread.lock().unwrap_or_else(PoisonError::into_inner).take() {
            let _ = thread.join();
        }
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;
    use std::sync::mpsc;

    use super::*;

    #[test]
    fn the_body_runs_for_each_wake_and_returns_on_quit() {
        let runs = Arc::new(AtomicUsize::new(0));
        let counted = runs.clone();
        let service = Service::start("test", move |waker| {
            while waker.wait(None) == Woke::Work {
                counted.fetch_add(1, Ordering::SeqCst);
            }
        })
        .unwrap();
        // The first wait returns at once (started woken); the others wait for a wake.
        while runs.load(Ordering::SeqCst) < 1 {
            std::thread::sleep(Duration::from_millis(1));
        }
        service.wake();
        service.wake();
        assert!(service.stop_within(Duration::from_secs(5)));
        assert!(runs.load(Ordering::SeqCst) >= 1);
    }

    #[test]
    fn an_idle_wait_times_out_without_work() {
        let (tx, rx) = mpsc::channel();
        let service = Service::start("test", move |waker| {
            let first = waker.wait(Some(Duration::from_millis(5)));
            let second = waker.wait(Some(Duration::from_millis(5)));
            let _ = tx.send((first, second));
        })
        .unwrap();
        // Started woken: the first wait is Work, the second, with nothing asked for, is Idle.
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), (Woke::Work, Woke::Idle));
        service.stop();
    }

    #[test]
    fn a_body_that_is_stuck_is_left_running_past_a_bounded_stop() {
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let service = Service::start("test", move |_| {
            let _ = release_rx.recv();
        })
        .unwrap();
        assert!(!service.stop_within(Duration::from_millis(20)));
        release_tx.send(()).unwrap();
        // Released right before the second stop: the stop waits for the body to return, within its bound.
        assert!(service.stop_within(Duration::from_secs(5)));
    }

    #[test]
    fn a_panicking_body_counts_as_exited() {
        let service = Service::start("test", |_| panic!("the body fails")).unwrap();
        assert!(service.stop_within(Duration::from_secs(5)));
    }
}
