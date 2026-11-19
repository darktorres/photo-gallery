//! The state of the `LaneModel` QObject: the rows the model holds, its two properties, and the fixture the screenshots read.

use pg_app::LaneLines;

use crate::lane_model::LaneRow;

#[derive(Default)]
#[expect(
    unreachable_pub,
    reason = "the bridge\'s generated code names it in a public interface (E0446), so it is `pub`"
)]
pub struct LaneRust {
    /// The rows as the model holds them, in order (the view's rows are these).
    pub(crate) rows: Vec<LaneRow>,
    /// Whether the rows stack (`lane_model::stacked`).
    pub(crate) stacked: bool,
    /// Whether a row waits on the hold alone (`lane_model::settling`): the model is asked again until it has gone.
    pub(crate) settling: bool,
    /// The fixture `--lane-rows` names, read once: the job's rows are not asked for while it is set, and the global pause.
    pub(crate) fixture: Option<(LaneLines, bool)>,
    /// Whether the fixture has been read (once, at the first refresh).
    pub(crate) loaded: bool,
}
