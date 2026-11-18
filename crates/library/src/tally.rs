//! The notices the lanes share. A notice is keyed by its pass or its kind, not by a lane (`Notice::lifecycle`), so one lane
//! that posts it overwrites another lane's, and one lane's clearing clears another's. The tally takes each lane's report
//! and says the user what all the lanes say together: a count is the sum of the lanes' counts, and a notice is cleared only
//! once every lane that holds it is clean.
//!
//! The notices kinds are the ones a lane raises: a pass that couldn't read some files, a round that failed, a thumbnail
//! cache that is full, and a failure ledger that couldn't be written. The others (a crop, a scan, a panic) are not lane
//! state, and are posted as they are.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use pg_jobs::NoticeSink;
use pg_model::notice::{ERROR_CAP, FilePass, Notice, Op, Trigger, tidy};
use pg_model::size::ThumbHeight;
use pg_thumbs::LedgerOutcome;

/// What the lanes hold, by lane number. A lane's entry is its latest report: a lane that reports again replaces it.
#[derive(Default)]
struct State {
    /// Per pass: each lane's count of the files it couldn't read, and whether its last pass read them all.
    reads: BTreeMap<FilePass, BTreeMap<usize, (usize, bool)>>,
    /// The lanes whose round failed, each with its error.
    rounds: BTreeMap<usize, String>,
    /// The lanes whose thumbnail cache is full, each with the height and the thumbnails it made.
    full: BTreeMap<usize, (ThumbHeight, usize)>,
    /// The lanes whose failure ledger couldn't be written, each with its error.
    ledger: BTreeMap<usize, String>,
}

/// The lanes' reports, and what the user is told of them.
pub(crate) struct Tally {
    notices: Arc<dyn NoticeSink>,
    state: Mutex<State>,
}

fn count(n: usize) -> u64 {
    u64::try_from(n).unwrap_or(u64::MAX)
}

impl Tally {
    pub(crate) fn new(notices: Arc<dyn NoticeSink>) -> Tally {
        Tally { notices, state: Mutex::new(State::default()) }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Lane `lane`'s pass read its files: `unreadable` of them couldn't be (they are tried again), and `complete` says the
    /// pass ran to its end. The pass's notice is the sum of the lanes' unreadable counts; it is resolved once no lane has
    /// one and every lane that ran the pass ended it.
    pub(crate) fn files_read(&self, lane: usize, pass: FilePass, unreadable: usize, complete: bool) {
        let mut state = self.state();
        state.reads.entry(pass).or_default().insert(lane, (unreadable, complete));
        self.settle_reads(&state, pass);
    }

    fn settle_reads(&self, state: &State, pass: FilePass) {
        let Some(lanes) = state.reads.get(&pass) else { return };
        let unreadable: usize = lanes.values().map(|(n, _)| *n).sum();
        if unreadable > 0 {
            self.notices.post(Notice::UnreadableFiles { pass, count: count(unreadable) });
        } else if lanes.values().all(|(_, complete)| *complete) {
            self.notices.resolve(Trigger::Pass(pass));
        }
    }

    /// Lane `lane`'s round ended: `failure` is its error, or `None` when the round ran. The round's notice is posted with the
    /// failing lane's error, and cleared when no lane's round has failed.
    pub(crate) fn round(&self, lane: usize, failure: Option<String>) {
        let mut state = self.state();
        match failure {
            Some(error) => {
                self.notices.post(Notice::RoundFailed { error: error.clone() });
                state.rounds.insert(lane, error);
            }
            None => {
                state.rounds.remove(&lane);
                if state.rounds.is_empty() {
                    self.notices.resolve(Trigger::Op(Op::Round));
                }
            }
        }
    }

    /// Lane `lane`'s thumbnail cache is full at `height`, after it made `made` thumbnails.
    pub(crate) fn cache_full(&self, lane: usize, height: ThumbHeight, made: usize) {
        let mut state = self.state();
        state.full.insert(lane, (height, made));
        let total: usize = state.full.values().map(|(_, n)| *n).sum();
        self.notices.post(Notice::CacheFull { height, made: count(total) });
    }

    /// Lane `lane`'s thumbnails pass ended without the cache being full: its own full-cache notice is no longer true.
    pub(crate) fn cache_room(&self, lane: usize) {
        let mut state = self.state();
        if state.full.remove(&lane).is_some() && state.full.is_empty() {
            self.notices.resolve(Trigger::Pass(FilePass::Thumbs));
        }
    }

    /// Lane `lane`'s failure ledger write came to `outcome`: a failure is posted; a write that went through clears the
    /// notice once no lane's failed.
    pub(crate) fn ledger(&self, lane: usize, outcome: LedgerOutcome) {
        let mut state = self.state();
        match outcome {
            LedgerOutcome::Failed(error) => {
                let error = tidy(&error, ERROR_CAP);
                self.notices.post(Notice::LedgerNotWritten { error: error.clone() });
                state.ledger.insert(lane, error);
            }
            LedgerOutcome::Written => {
                if state.ledger.remove(&lane).is_some() && state.ledger.is_empty() {
                    self.notices.resolve(Trigger::Op(Op::LedgerWrite(FilePass::Thumbs)));
                }
            }
            LedgerOutcome::Untouched => {}
        }
    }

    /// Lane `lane` was retired: it holds nothing any more, so the notices that it alone held are settled as the others are.
    pub(crate) fn lane_retired(&self, lane: usize) {
        let mut state = self.state();
        let passes: Vec<FilePass> = state.reads.keys().copied().collect();
        for pass in passes {
            if let Some(lanes) = state.reads.get_mut(&pass) {
                lanes.remove(&lane);
            }
            self.settle_reads(&state, pass);
        }
        if state.rounds.remove(&lane).is_some() && state.rounds.is_empty() {
            self.notices.resolve(Trigger::Op(Op::Round));
        }
        if state.full.remove(&lane).is_some() && state.full.is_empty() {
            self.notices.resolve(Trigger::Pass(FilePass::Thumbs));
        }
        if state.ledger.remove(&lane).is_some() && state.ledger.is_empty() {
            self.notices.resolve(Trigger::Op(Op::LedgerWrite(FilePass::Thumbs)));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use pg_jobs::NoticeSink;
    use pg_model::notice::{Notice, Trigger};

    use super::*;

    /// Records what the tally posts and resolves, in order.
    #[derive(Default)]
    struct Recording(Mutex<Vec<String>>);

    impl NoticeSink for Recording {
        fn post(&self, notice: Notice) {
            self.0.lock().unwrap().push(format!("post {notice:?}"));
        }
        fn replace(&self, _scope: pg_model::notice::KeyBuf, _notices: Vec<Notice>) {}
        fn resolve(&self, trigger: Trigger) {
            self.0.lock().unwrap().push(format!("resolve {trigger:?}"));
        }
        fn poke(&self) {}
    }

    fn tally() -> (Tally, Arc<Recording>) {
        let sink = Arc::new(Recording::default());
        (Tally::new(sink.clone()), sink)
    }

    fn said(sink: &Recording) -> Vec<String> {
        std::mem::take(&mut *sink.0.lock().unwrap())
    }

    fn unreadable(count: u64) -> String {
        format!("post UnreadableFiles {{ pass: Import, count: {count} }}")
    }

    #[test]
    fn the_unreadable_count_of_a_pass_is_the_sum_of_the_lanes() {
        let (tally, sink) = tally();
        tally.files_read(0, FilePass::Import, 2, false);
        tally.files_read(1, FilePass::Import, 3, false);
        assert_eq!(said(&sink), vec![unreadable(2), unreadable(5)]);
    }

    #[test]
    fn one_lanes_clean_pass_does_not_clear_another_lanes_unreadable_files() {
        let (tally, sink) = tally();
        tally.files_read(0, FilePass::Import, 4, true);
        said(&sink);
        tally.files_read(1, FilePass::Import, 0, true);
        // Lane 0 still can't read four files: the notice stays, with the same count.
        assert_eq!(said(&sink), vec![unreadable(4)]);
    }

    #[test]
    fn the_unreadable_notice_resolves_once_every_lane_that_ran_the_pass_is_clean() {
        let (tally, sink) = tally();
        tally.files_read(1, FilePass::Import, 0, false);
        assert!(said(&sink).is_empty(), "lane 1 was stopped: nothing is resolved");
        tally.files_read(0, FilePass::Import, 0, true);
        assert!(said(&sink).is_empty(), "lane 1 has not ended its pass: the notice stays");
        tally.files_read(1, FilePass::Import, 0, true);
        assert_eq!(said(&sink), vec!["resolve Pass(Import)".to_string()]);
    }

    #[test]
    fn a_failed_round_is_cleared_only_when_every_failed_lane_has_run_again() {
        let (tally, sink) = tally();
        tally.round(0, Some("boom".to_string()));
        tally.round(1, Some("bang".to_string()));
        said(&sink);
        tally.round(0, None);
        assert!(said(&sink).is_empty(), "lane 1's round still failed");
        tally.round(1, None);
        assert_eq!(said(&sink), vec!["resolve Op(Round)".to_string()]);
    }

    #[test]
    fn a_retired_lane_stops_holding_its_notices() {
        let (tally, sink) = tally();
        tally.files_read(0, FilePass::Import, 2, false);
        tally.files_read(1, FilePass::Import, 0, true);
        said(&sink);
        tally.lane_retired(0);
        assert_eq!(said(&sink), vec!["resolve Pass(Import)".to_string()]);
    }

    #[test]
    fn a_full_cache_is_cleared_by_the_last_lane_to_have_room() {
        let (tally, sink) = tally();
        let height = ThumbHeight::DEFAULT;
        tally.cache_full(0, height, 3);
        tally.cache_full(1, height, 4);
        said(&sink);
        tally.cache_room(0);
        assert!(said(&sink).is_empty());
        tally.cache_room(1);
        assert_eq!(said(&sink), vec!["resolve Pass(Thumbs)".to_string()]);
    }
}
