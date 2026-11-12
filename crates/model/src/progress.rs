//! How far a running pass is: its phase and its counts, as data the job layer reports (`pg_jobs::control`) and the front
//! ends word (`pg_client::text::pass_status`, MODULARITY_REVIEW_4 R-1 and M-45). The time is the elapsed time when the
//! progress was read, so nothing here reads a clock.

use std::time::Duration;

use crate::num;
use crate::rules::Path as RulePath;
use crate::size::ThumbHeight;

/// The phase a pass is in, and what it works on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Phase {
    Idle,
    Crop,
    /// The folder being scanned, under the catalog's rules.
    Scan {
        root: RulePath,
    },
    Import,
    Borders,
    /// The thumbnails made at this height.
    Thumbs {
        height: ThumbHeight,
    },
}

/// A pass's progress: its phase, its files done and in all, the failures and bytes so far, and how long it has run.
#[derive(Clone, Debug, PartialEq)]
pub struct Progress {
    pub phase: Phase,
    pub done: u64,
    /// 0 when unknown (scanning).
    pub total: u64,
    pub failed: u64,
    pub bytes: u64,
    /// How long the phase had run when this was read.
    pub elapsed: Duration,
}

impl Progress {
    /// No pass running.
    pub fn idle() -> Progress {
        Progress { phase: Phase::Idle, done: 0, total: 0, failed: 0, bytes: 0, elapsed: Duration::ZERO }
    }

    /// Files per second so far.
    pub fn rate(&self) -> f64 {
        num::size(self.done) / self.elapsed.as_secs_f64().max(1e-3)
    }

    /// Seconds left at the rate so far (0 when there is no rate yet or nothing is known).
    pub fn eta(&self) -> f64 {
        let rate = self.rate();
        if rate > 0.0 { num::size(self.total.saturating_sub(self.done)) / rate } else { 0.0 }
    }

    /// Done over total, 0 when the total is unknown.
    pub fn fraction(&self) -> f64 {
        if self.total == 0 { 0.0 } else { num::size(self.done) / num::size(self.total) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rate_and_the_time_left_come_from_the_counts_and_the_elapsed_time() {
        let p = Progress {
            phase: Phase::Import,
            done: 100,
            total: 400,
            failed: 0,
            bytes: 0,
            elapsed: Duration::from_secs(4),
        };
        assert_eq!(p.rate(), 25.0);
        assert_eq!(p.eta(), 12.0);
        assert_eq!(p.fraction(), 0.25);
        assert_eq!(Progress::idle().eta(), 0.0, "no rate yet");
        assert_eq!(Progress { total: 0, ..p }.fraction(), 0.0, "an unknown total");
    }
}
