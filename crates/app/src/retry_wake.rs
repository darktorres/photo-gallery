//! The wake for a Retry the inbox replayed at the app's start: the library's job is woken for the files that Retry marked.
//! The replay and the library's start can finish in either order, so both go through one lock: whichever comes second
//! does the wake, and a wake is never lost.

use std::sync::{Mutex, PoisonError};

use pg_library::runner::JobWaker;

#[derive(Default)]
pub(crate) struct RetryWake {
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    /// A replayed Retry that has not reached a job yet.
    owed: bool,
    /// The job's waker, once the library has started.
    job: Option<JobWaker>,
}

impl RetryWake {
    /// The inbox replayed a Retry: the job is woken now if it has started, or owed to it when it starts.
    pub(crate) fn replayed(&self) {
        let job = {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            match &state.job {
                Some(job) => Some(job.clone()),
                None => {
                    state.owed = true;
                    None
                }
            }
        };
        if let Some(job) = job {
            job.wake();
        }
    }

    /// The library's job has started: it is the wake from now on, and a replay that came before it is delivered.
    pub(crate) fn start(&self, job: JobWaker) {
        let owed = {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            state.job = Some(job.clone());
            std::mem::take(&mut state.owed)
        };
        if owed {
            job.wake();
        }
    }

    /// Whether a replayed Retry is still owed to a job that has not started.
    #[cfg(test)]
    pub(crate) fn owed(&self) -> bool {
        self.state.lock().unwrap_or_else(PoisonError::into_inner).owed
    }
}
