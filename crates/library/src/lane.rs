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
use pg_jobs::schedule::{self, Step};
use pg_jobs::service::{Waker, Woke};
use pg_model::notice::{ERROR_CAP, Notice, tidy};
use pg_platform::native::PATH_RULES;

use crate::inner::Inner;
use crate::passes::{self, Cx};

/// How long a walk refused by another walk's lease (an overlapping folder, in this process or another) waits before it
/// is tried again, with nothing else to wake the lane: a lease ends with its walk, or expires in
/// `pg_catalog::SCAN_LEASE_SECS`.
pub(crate) const BUSY_RETRY: Duration = Duration::from_secs(5);

/// Why a round failed: an error, or a panic (which the lane reports, and goes on).
enum Fault {
    Failed(String),
    Panicked(String),
}

impl Inner {
    /// The lane's thread: rounds, one after another, each started by a request, a wake or the busy retry.
    pub(crate) fn run(&self, waker: &Waker<'_>) {
        // One connection for every round; after a round that failed, a new one.
        let mut catalog: Option<Catalog> = None;
        // A round stopped by a new request starts the next one at once: the request is already queued.
        let mut again = false;
        loop {
            if !again {
                let idle = (!self.busy().is_empty()).then_some(BUSY_RETRY);
                if waker.wait(idle) == Woke::Quit {
                    return;
                }
            }
            again = false;
            // A held lane waits for its hold to end: a crop or a cut works on its volumes meanwhile. A request that stopped
            // the lane while it waited is cleared below, with the requests' stops: each request queued its work.
            self.ctl.wait_while_held(&|| waker.quitting());
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
            match outcome {
                Ok(step) => {
                    // A round that ran is the end of a failed one's notice.
                    self.shared.tally.round(self.number, None);
                    again = matches!(step, Step::Preempted);
                }
                Err(Fault::Failed(error)) => self.shared.tally.round(self.number, Some(error)),
                Err(Fault::Panicked(message)) => {
                    self.notify(Notice::RoundPanicked { message: tidy(&message, ERROR_CAP) });
                }
            }
            self.shared.round_ended();
        }
    }

    /// A round, with a panic in it caught: the lane reports it (the inbox, and the log through the panic hook) and goes on
    /// with the next request.
    fn round_caught(&self, catalog: &mut Option<Catalog>) -> std::result::Result<Step, Fault> {
        match panics::catch(AssertUnwindSafe(|| self.round(catalog))) {
            Ok(result) => result.map_err(|e| Fault::Failed(tidy(&pg_model::error::chain(&e), ERROR_CAP))),
            Err(panic) => {
                // The connection a panicking pass held is not trusted: the next round opens a new one.
                *catalog = None;
                Err(Fault::Panicked(panics::message(&*panic).to_string()))
            }
        }
    }

    /// One round: the passes in order, on the connection the lane keeps between rounds.
    pub(crate) fn round(&self, conn: &mut Option<Catalog>) -> Result<Step> {
        // The round's span in the perf log: the passes it runs are its children, and the catalog's open is timed apart.
        let _round = pg_perf::span("round");
        let open = pg_perf::span("round.open");
        let mut catalog = match conn.take() {
            Some(c) => c,
            None => Catalog::open(&self.shared.catalog_path, PATH_RULES)?,
        };
        drop(open);
        let step = {
            let mut cx = Cx { inner: self, catalog: &mut catalog };
            schedule::round(
                &mut [
                    &mut passes::Scans,
                    &mut passes::Import,
                    &mut passes::Thumbs,
                    &mut passes::Placeholders,
                    &mut passes::Hashes,
                    &mut passes::Pairing,
                    &mut passes::Borders,
                ],
                &mut cx,
                |cx| cx.inner.ctl.stop_requested(),
            )?
        };
        *conn = Some(catalog);
        Ok(step)
    }
}
