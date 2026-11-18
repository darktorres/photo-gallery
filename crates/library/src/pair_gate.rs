//! The pairing pass's gate, shared by the lanes: a leaf (it names nothing of the job), so the shared state can hold it
//! without the passes that run the join.

use std::sync::{Mutex, MutexGuard, PoisonError};

/// The pairing pass's gate: one lane runs the duplicates' join at a time, and a lane that wants one while another runs
/// makes the runner run it once more after (the other's join may not have the rows this lane stored).
#[derive(Default)]
pub(crate) struct PairGate {
    state: Mutex<PairState>,
}

#[derive(Default)]
struct PairState {
    /// A lane is running the join.
    running: bool,
    /// A join is wanted after the one running (or now).
    wanted: bool,
}

impl PairGate {
    fn state(&self) -> MutexGuard<'_, PairState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A lane wants a join. `true` when it runs one (no other lane does); `false` when another lane does, and will run
    /// the wanted one after its own.
    pub(crate) fn begin(&self) -> bool {
        let mut state = self.state();
        state.wanted = true;
        if state.running {
            return false;
        }
        state.running = true;
        true
    }

    /// The runner's next join, if one is wanted (and it takes the want); otherwise the runner is done.
    pub(crate) fn next(&self) -> bool {
        let mut state = self.state();
        if state.wanted {
            state.wanted = false;
            return true;
        }
        state.running = false;
        false
    }

    /// The runner stops before its wanted join is done (a stop, a failure): the join stays wanted for the next runner.
    pub(crate) fn abandon(&self) {
        let mut state = self.state();
        state.running = false;
        state.wanted = true;
    }
}
