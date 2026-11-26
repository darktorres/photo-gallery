//! One lane of the background job: a thread that runs the round of passes over its volumes, one lane per drive (`lanes`).
//! The coordinator (`coordinator`) makes the lanes, gives each its volumes and its scans, and retires them; a lane only
//! does its round. The round is the passes in order (`passes`): the folders asked for are walked first, then the files are
//! imported, thumbnailed, placed, hashed, paired, and their borders looked for.
//!
//! A lane stops its round at a request (`Control::request_stop`), and waits while it is held (`Control::hold`): a crop or
//! a cut that changes this lane's volumes holds the lane, and waits until the round in progress has ended, before it works.

use std::panic::AssertUnwindSafe;
use std::time::Duration;

use crate::Result;
use pg_catalog::Catalog;
use pg_jobs::panics;
use pg_jobs::schedule::{self, Pass, Step};
use pg_jobs::service::{Waker, Woke};
use pg_model::notice::{ERROR_CAP, tidy};
use pg_platform::native::PATH_RULES;

use crate::inner::Inner;
use crate::mask::PassKind;
use crate::passes::{self, Cx};
use crate::tally::RoundEnd;

/// How long a walk refused by another walk's lease (an overlapping folder, in this process or another) waits before it
/// is tried again, with nothing else to wake the lane: a lease ends with its walk, or expires in
/// `pg_catalog::SCAN_LEASE_SECS`.
pub(crate) const BUSY_RETRY: Duration = Duration::from_secs(5);

/// The first wait after a failed round (section 3); each failure in a row doubles it, up to [`BACKOFF_CAP_SECS`].
const BACKOFF_FIRST_SECS: u64 = 5;
/// The longest wait after failed rounds: a failure that stays is tried this often, with no give-up (goal 3).
const BACKOFF_CAP_SECS: u64 = 60;

/// The wait after `failures` failed rounds in a row: 5, 10, 20, 40 and then 60 seconds, from then on. None after none.
pub(crate) fn failure_backoff(failures: u32) -> Option<Duration> {
    let doublings = failures.checked_sub(1)?;
    let factor = 1u64.checked_shl(doublings.min(16)).unwrap_or(u64::MAX);
    Some(Duration::from_secs(BACKOFF_FIRST_SECS.saturating_mul(factor).min(BACKOFF_CAP_SECS)))
}

/// The earliest of two deadlines, either of which may be absent.
fn earliest(a: Option<Duration>, b: Option<Duration>) -> Option<Duration> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, None) => a,
        (None, b) => b,
    }
}

/// Why a round failed: an error, a panic (which the lane reports, and goes on), or an error a pass has already said.
enum Fault {
    Failed(String),
    Panicked(String),
    Said,
}

/// A round that returned: how it ended, and whether a pass said a failure of its own (`Cx::said`), which makes the round a
/// failed one for the backoff, with no notice of the round's.
pub(crate) struct Ran {
    pub(crate) step: Step,
    pub(crate) said: bool,
}

impl Inner {
    /// The lane's thread: rounds, one after another, each started by a request, a wake, the busy retry or the backoff
    /// after a failed round.
    pub(crate) fn run(&self, waker: &Waker<'_>) {
        // One connection for every round; after a round that failed, a new one.
        let mut catalog: Option<Catalog> = None;
        // A round stopped by a new request starts the next one at once: the request is already queued.
        let mut again = false;
        // The rounds failed in a row: the next is tried after the backoff they make, or at once on a wake. A round that
        // runs to its end resets it; a preempted one leaves it.
        let mut failures: u32 = 0;
        loop {
            if !again {
                // Going idle: what the lane learned of its drive is kept (when it learned something new).
                self.save_profiles();
                // One deadline: the busy retry while a walk waits for a lease, the backoff while rounds fail.
                let busy = (!self.busy().is_empty()).then_some(BUSY_RETRY);
                let idle = earliest(busy, failure_backoff(failures));
                if waker.wait(idle) == Woke::Quit {
                    return;
                }
            }
            again = false;
            // A held lane waits for its hold to end: a crop or a cut works on its volumes meanwhile. A request that stopped
            // the lane while it waited is cleared below, with the requests' stops: each request queued its work.
            self.ctl.wait_while_held(&|| waker.quitting());
            // A removal parks the lanes on its folder's volumes until it has hidden the folder's files: a lane on one waits
            // here, before any work (section 1, D5 and D6). A quit ends the lane.
            if !self.shared.parks.wait_clear(&self.volume_set(), &|| waker.quitting()) {
                return;
            }
            // The stop the last request set is cleared here, then the quit is looked for: a quit asked after this check
            // comes with a stop after the clear, so the round stops.
            self.ctl.clear_stop();
            if waker.quitting() {
                return;
            }
            self.requeue_busy();
            self.set_active(true);
            let outcome = self.round_caught(&mut catalog);
            self.set_active(false);
            let end = match outcome {
                // A round that ran to its end is the end of a failed one's notice, and of the backoff.
                Ok(Step::Done) => {
                    failures = 0;
                    RoundEnd::Done
                }
                // A preempted round is not a failure, and clears none: the next round runs at once.
                Ok(Step::Preempted) => {
                    again = true;
                    self.shared.round_ended();
                    continue;
                }
                Err(fault) => {
                    failures = failures.saturating_add(1);
                    match fault {
                        Fault::Failed(error) => RoundEnd::Failed(error),
                        Fault::Panicked(message) => RoundEnd::Panicked(message),
                        Fault::Said => RoundEnd::Said,
                    }
                }
            };
            self.shared.tally.round(self.number, end);
            self.shared.round_ended();
        }
    }

    /// A round, with a panic in it caught: the lane reports it (the inbox through the tally, and the log through the panic
    /// hook) and goes on with the next request. A round whose pass said its own failure is a failure too (`Fault::Said`).
    fn round_caught(&self, catalog: &mut Option<Catalog>) -> std::result::Result<Step, Fault> {
        match panics::catch(AssertUnwindSafe(|| self.round(catalog))) {
            Ok(Ok(Ran { said: true, .. })) => Err(Fault::Said),
            Ok(Ok(Ran { step, said: false })) => Ok(step),
            Ok(Err(e)) => Err(Fault::Failed(tidy(&pg_model::error::chain(&e), ERROR_CAP))),
            Err(panic) => {
                // The connection a panicking pass held is not trusted: the next round opens a new one.
                *catalog = None;
                Err(Fault::Panicked(panics::message(&*panic).to_string()))
            }
        }
    }

    /// One round: the passes in order, on the connection the lane keeps between rounds. A pass that fails does not stop
    /// the others (`schedule::round`); the round returns the first error.
    pub(crate) fn round(&self, conn: &mut Option<Catalog>) -> Result<Ran> {
        // The round's span in the perf log: the passes it runs are its children, and the catalog's open is timed apart.
        let _round = pg_perf::span("round");
        let open = pg_perf::span("round.open");
        let mut catalog = match conn.take() {
            Some(c) => c,
            None => Catalog::open(&self.shared.catalog_path, PATH_RULES)?,
        };
        drop(open);
        #[cfg(test)]
        self.shared.hooks.at_round(&self.volume_set());
        let mut said = false;
        let step = {
            let mut cx = Cx { inner: self, catalog: &mut catalog, said: &mut said };
            // The passes this round runs: the set the job holds now (`Runner::set_passes`), in round order. A pass left out is
            // not run, and what it owes stays owed for the next round that runs it.
            let mask = self.shared.passes.get();
            let (mut scans, mut import, mut thumbs, mut placeholders) =
                (passes::Scans, passes::Import, passes::Thumbs, passes::Placeholders);
            let (mut hashes, mut pairing, mut borders) = (passes::Hashes, passes::Pairing, passes::Borders);
            let mut list: Vec<&mut dyn Pass<Cx<'_>>> = Vec::with_capacity(PassKind::ALL.len());
            if mask.contains(PassKind::Scans) {
                list.push(&mut scans);
            }
            if mask.contains(PassKind::Import) {
                list.push(&mut import);
            }
            if mask.contains(PassKind::Thumbs) {
                list.push(&mut thumbs);
            }
            if mask.contains(PassKind::Placeholders) {
                list.push(&mut placeholders);
            }
            if mask.contains(PassKind::Hashes) {
                list.push(&mut hashes);
            }
            if mask.contains(PassKind::Pairing) {
                list.push(&mut pairing);
            }
            if mask.contains(PassKind::Borders) {
                list.push(&mut borders);
            }
            schedule::round(&mut list, &mut cx, |cx| cx.inner.ctl.stop_requested())
        };
        // The round's writes are in the WAL: this thread moves them into the file at the end of its work, so no writer's
        // commit pays for the checkpoint. A round that failed is checkpointed too.
        let checked = catalog.checkpoint_passive();
        *conn = Some(catalog);
        let step = step?;
        checked?;
        Ok(Ran { step, said })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The backoff after failed rounds in a row: none before a failure, then 5, 10, 20, 40 and 60 seconds, and 60 from
    /// then on, with no give-up and no overflow at any count.
    #[test]
    fn the_backoff_doubles_from_five_seconds_to_a_minute_and_stays() {
        let secs = |n| failure_backoff(n).map(|d| d.as_secs());
        assert_eq!(secs(0), None);
        assert_eq!([1, 2, 3, 4, 5, 6].map(secs), [5, 10, 20, 40, 60, 60].map(Some));
        assert_eq!(secs(u32::MAX), Some(60));
    }

    /// The lane's deadline is the earliest of the busy retry and the backoff, either of which may be absent.
    #[test]
    fn the_deadline_is_the_earliest_of_the_two() {
        let s = Duration::from_secs;
        assert_eq!(earliest(Some(s(5)), Some(s(60))), Some(s(5)));
        assert_eq!(earliest(None, Some(s(10))), Some(s(10)));
        assert_eq!(earliest(Some(s(5)), None), Some(s(5)));
        assert_eq!(earliest(None, None), None);
    }
}
