//! The state of the `LaneModel` QObject: the rows the model holds, its two properties, and the fixture the screenshots read.

use pg_app::LaneLines;

/// A row's identity: the crop thread (first, always) or a lane by its number (`LaneProgress::number`, never reused).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum RowKey {
    Crop,
    Lane(usize),
}

/// One row, as the model holds it: every role is a field, so a change of any of them is a `dataChanged` of the row.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LaneRow {
    pub(crate) key: RowKey,
    /// The lane's number, -1 for the crop row (`RowKey::Crop`).
    pub(crate) number: i32,
    /// Whether the row has a Pause: a lane's own pause. The crop thread has none.
    pub(crate) pausable: bool,
    /// The drive's label, empty when it can't be told, and for the crop row.
    pub(crate) label: String,
    /// The pass's line without the label, or the paused wording while the lane is paused itself.
    pub(crate) status: String,
    /// The pass's failed count ("2 failed"), empty when none.
    pub(crate) failed: String,
    /// The ledger the failed files are listed under (`ledgers::kind_code`), -1 when the pass has none.
    pub(crate) failed_pass: i32,
    /// The lane's own pause (what its button shows and sets).
    pub(crate) paused: bool,
    /// The lane is in a phase now: its bar shows.
    pub(crate) working: bool,
    /// Done over total, 0 when the total is unknown.
    pub(crate) progress: f64,
    /// The total is unknown: the bar is indeterminate while it runs.
    pub(crate) indeterminate: bool,
}

#[derive(Default)]
#[expect(
    unreachable_pub,
    reason = "the bridge\'s generated code names it in a public interface (E0446), so it is `pub`"
)]
pub struct LaneRust {
    /// The rows as the model holds them, in order (the view's rows are these).
    pub(crate) rows: Vec<LaneRow>,
    /// Whether a row waits on the hold alone (`lane_model::settling`): the model is asked again until it has gone.
    pub(crate) settling: bool,
    /// The fixture `--lane-rows` names, read once: the job's rows are not asked for while it is set.
    pub(crate) fixture: Option<LaneLines>,
    /// Whether the fixture has been read (once, at the first refresh).
    pub(crate) loaded: bool,
}
