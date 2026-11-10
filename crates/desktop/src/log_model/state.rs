//! The state of the `LogModel` QObject: the bridge's backing struct (the records shown).

use std::collections::VecDeque;

#[derive(Default)]
#[expect(
    unreachable_pub,
    reason = "the bridge\'s generated code names it in a public interface (E0446), so it is `pub`"
)]
pub struct LogRust {
    pub(crate) ready: bool,
    /// The records shown, oldest first: at most `pg_log::RING_RECORDS`.
    pub(super) rows: VecDeque<pg_log::Record>,
    /// `start` ran: the feed is followed.
    pub(super) started: bool,
}
