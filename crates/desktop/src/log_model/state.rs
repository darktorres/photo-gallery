//! The state of the `LogModel` QObject: the bridge's backing struct (the records shown).

use std::collections::VecDeque;

#[derive(Default)]
#[expect(
    unreachable_pub,
    reason = "the bridge's generated code names it in a public interface (E0446), so it is `pub`"
)]
pub struct LogRust {
    pub(crate) ready: bool,
    /// The records shown, oldest first: at most `pg_log::RING_RECORDS`.
    pub(super) rows: VecDeque<pg_log::Record>,
    /// The number (the `seq` role) of the first row: how many rows were trimmed from the front since the records were
    /// read. Row `i` is number `first + i`.
    pub(super) first: u64,
    /// `start` ran: the feed is followed.
    pub(super) started: bool,
}

impl LogRust {
    /// The records shown become `records`, oldest first, numbered from 0.
    pub(crate) fn reset(&mut self, records: Vec<pg_log::Record>) {
        self.rows = records.into();
        self.first = 0;
    }

    /// The number (the `seq` role) of row `row`: `first` plus its place.
    pub(crate) fn seq_of(&self, row: usize) -> u64 {
        self.first.saturating_add(u64::try_from(row).unwrap_or(u64::MAX))
    }

    /// How many of the oldest rows are past the cap, and go.
    pub(crate) fn over_cap(&self) -> usize {
        self.rows.len().saturating_sub(pg_log::RING_RECORDS)
    }

    /// Drops the `count` oldest rows. The rows left keep their numbers: `first` counts the dropped ones.
    pub(crate) fn drop_oldest(&mut self, count: usize) {
        self.rows.drain(..count);
        self.first = self.first.saturating_add(u64::try_from(count).unwrap_or(u64::MAX));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pg_model::notice::Level;

    fn record(message: &str) -> pg_log::Record {
        pg_log::Record {
            t_ms: None,
            local: "2026-11-10 12:00:00".into(),
            level: Level::Info,
            target: "session".into(),
            message: message.into(),
            detail: None,
            pid: 0,
            session: 0,
            seq: 0,
            file_only: false,
        }
    }

    /// `count` rows whose messages are their places, numbered from `first`.
    fn with_rows(count: usize, first: u64) -> LogRust {
        LogRust { rows: (0..count).map(|i| record(&i.to_string())).collect(), first, ..LogRust::default() }
    }

    #[test]
    fn a_reset_numbers_the_records_from_zero() {
        let mut log = with_rows(4, 9);
        log.reset(vec![record("a"), record("b")]);
        assert_eq!((log.seq_of(0), log.seq_of(1)), (0, 1));
        assert_eq!(log.rows.len(), 2);
    }

    #[test]
    fn a_row_keeps_its_number_when_the_rows_before_it_go() {
        let mut log = with_rows(5, 0);
        assert_eq!(log.seq_of(3), 3);
        log.drop_oldest(2);
        assert_eq!(log.rows.len(), 3);
        assert_eq!(log.rows.front().map(|r| r.message.as_str()), Some("2"));
        assert_eq!(log.seq_of(0), 2, "the row that was 2 is still 2");
        assert_eq!(log.seq_of(2), 4);
    }

    #[test]
    fn the_cap_drops_the_oldest_and_the_numbers_go_on_from_them() {
        let mut log = with_rows(pg_log::RING_RECORDS + 3, 0);
        assert_eq!(log.over_cap(), 3);
        let last = log.seq_of(log.rows.len() - 1);
        log.drop_oldest(3);
        assert_eq!(log.over_cap(), 0);
        assert_eq!(log.rows.len(), pg_log::RING_RECORDS);
        assert_eq!(log.seq_of(0), 3);
        assert_eq!(log.seq_of(log.rows.len() - 1), last, "the newest row keeps its number");
    }

    #[test]
    fn a_log_under_the_cap_drops_nothing() {
        let log = with_rows(pg_log::RING_RECORDS, 0);
        assert_eq!(log.over_cap(), 0);
    }
}
