//! The pauses the window waits out (S-12.18): a query typed is shown after a short rest, the session is saved once the
//! selection rests, the grid re-lays out once a resize settles, the thumbnails follow a size change. Each is a deadline
//! on a monotonic `now_ms` the host passes in (no clock is read here, so the tests run in virtual time). Arming a timer
//! that is already armed moves its deadline (the pause starts again). The host runs one timer, set to `next_deadline`,
//! and calls `handle_timeout` when it fires; a late or early call fires only the deadlines that are due.
use serde::{Deserialize, Serialize};

/// A pause the window waits out.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Timer {
    /// The filter bar's query rests: the list is shown (300 ms).
    ApplyQuery,
    /// The selection rests: the session's last file is saved (1 s).
    SaveSession,
    /// The grid's width settled: the grid is laid out again (50 ms).
    Layout,
    /// The row height settled: the thumbnails follow and the row height is saved (300 ms).
    Size,
}

impl Timer {
    /// How long the pause lasts, in milliseconds.
    pub fn delay_ms(self) -> u64 {
        match self {
            Timer::ApplyQuery | Timer::Size => 300,
            Timer::SaveSession => 1000,
            Timer::Layout => 50,
        }
    }
}

/// The armed pauses, each with its deadline.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Deadlines {
    due: Vec<(Timer, u64)>,
}

impl Deadlines {
    /// Start `timer`'s pause at `now_ms` (or start it again, when it is armed).
    pub fn arm(&mut self, timer: Timer, now_ms: u64) {
        self.due.retain(|(t, _)| *t != timer);
        self.due.push((timer, now_ms + timer.delay_ms()));
    }

    /// Whether any pause is running (the screenshot waits for none).
    pub fn pending(&self) -> bool {
        !self.due.is_empty()
    }

    /// The earliest deadline, if any: the host sets its timer to it.
    pub fn next_deadline(&self) -> Option<u64> {
        self.due.iter().map(|(_, at)| *at).min()
    }

    /// The pauses due at `now_ms`, in the order they were due. Each fired pause is no longer armed.
    pub fn handle_timeout(&mut self, now_ms: u64) -> Vec<Timer> {
        let mut fired: Vec<(Timer, u64)> = self.due.iter().filter(|(_, at)| *at <= now_ms).copied().collect();
        self.due.retain(|(_, at)| *at > now_ms);
        fired.sort_by_key(|(_, at)| *at);
        fired.into_iter().map(|(timer, _)| timer).collect()
    }

    /// Every armed pause, at once, in the order they were due (the shutdown flush: pending saves are never lost).
    pub fn drain(&mut self) -> Vec<Timer> {
        let mut all = std::mem::take(&mut self.due);
        all.sort_by_key(|(_, at)| *at);
        all.into_iter().map(|(timer, _)| timer).collect()
    }
}
