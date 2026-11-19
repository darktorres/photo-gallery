//! The lanes the status bar shows, one row each (`StatusBar.qml` stacks them, `LaneRow.qml` shows one): a list model over the
//! job's rows (`LaneLines`), keyed by the crop thread and each lane's number. A row keeps its delegate, its bar and a Pause
//! press in flight when the rows around it change, since a change is an insert, a remove or a `dataChanged`, never a reset.
//!
//! The rows are a pure function of what the library reports (`rows_of`). Nothing here keeps a clock: a lane's row lasts
//! while the lane had work less than `HOLD` ago, which the library measures (`LaneProgress::since_work`). `settling` says
//! when a row exists only because of that hold, so the window asks again until it has gone (`Main.qml`'s timer).
//!
//! With `--lane-rows=<file>` (the screenshots' fixed lanes) the rows come from the file, through the same `rows_of`, and
//! the library is not asked.

pub(crate) mod state;

use crate::lane_model_bridge::qobject;

use core::pin::Pin;
use std::time::Duration;

use cxx_qt::CxxQtType;
use cxx_qt_lib::{QByteArray, QHash, QHashPair_i32_QByteArray, QList, QModelIndex, QString, QVariant};
use pg_app::LaneLines;
use pg_app::reexport::LaneProgress;
use pg_client::text;
use pg_model::notice::{FilePass, LedgerKind};
use pg_model::progress::{Mark, Phase, Progress};
use pg_model::rules::PathRules;
use pg_model::size::ThumbHeight;

use crate::command_line;
use crate::ledgers::kind_code;
use crate::library::running;
use crate::list_diff::{self, Op};

/// How long a lane's row lasts after its last work: longer than a pass's usual non-phase prelude (the pairing's load, the
/// thumbnails' cache check), so those don't blink the row. Tuned in the run.
pub(crate) const HOLD: Duration = Duration::from_secs(3);

const ROLE_NUMBER: i32 = 256;
const ROLE_PAUSABLE: i32 = 257;
const ROLE_LABEL: i32 = 258;
const ROLE_STATUS: i32 = 259;
const ROLE_PAUSED: i32 = 260;
const ROLE_WORKING: i32 = 261;
const ROLE_PROGRESS: i32 = 262;
const ROLE_INDETERMINATE: i32 = 263;
const ROLE_FAILED: i32 = 264;
const ROLE_FAILED_PASS: i32 = 265;

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

/// The `number` of a key: the lane's, or -1 for the crop row.
fn number_of(key: RowKey) -> i32 {
    match key {
        RowKey::Crop => -1,
        RowKey::Lane(number) => i32::try_from(number).unwrap_or(i32::MAX),
    }
}

/// The ledger a pass's failed files are listed under: the thumbnails', the borders', the import's (a file that doesn't
/// decode is recorded there: the placeholders' failures are the thumbnails', the hashes' the import's), or -1 for a pass
/// with none (`ledgers::kind_code`).
fn failed_pass_of(phase: &Phase) -> i32 {
    let pass = match phase {
        Phase::Thumbs { .. } | Phase::Placeholders => Some(FilePass::Thumbs),
        Phase::Borders => Some(FilePass::Borders),
        Phase::Import | Phase::Hashes => Some(FilePass::Import),
        Phase::Idle | Phase::Scan { .. } | Phase::Crop | Phase::Pairing => None,
    };
    kind_code(pass.map(LedgerKind::File))
}

/// One row from a progress: its status is the pass's line, or the paused wording while the lane is paused itself.
fn row_of(key: RowKey, label: String, progress: &Progress, paused: bool, rules: PathRules) -> LaneRow {
    let known = progress.known_fraction();
    let status = if paused { text::lane_paused_status(progress, rules) } else { text::pass_status(progress, rules) };
    LaneRow {
        key,
        number: number_of(key),
        pausable: matches!(key, RowKey::Lane(_)),
        label,
        status,
        failed: text::pass_failed(progress),
        failed_pass: failed_pass_of(&progress.phase),
        paused,
        working: progress.phase != Phase::Idle,
        progress: known.unwrap_or(0.0),
        indeterminate: known.is_none(),
    }
}

/// The rows the job's lines show now, in order: the crop's first, then the lanes' by number. The crop's row shows while it
/// had work less than `HOLD` ago; a lane's while it is paused itself, or had work less than `HOLD` ago. A lane never
/// given a row by the library (`since_work` `None`, not paused) has none.
pub(crate) fn rows_of(lines: &LaneLines, rules: PathRules) -> Vec<LaneRow> {
    let mut rows = Vec::new();
    if lines.crop_since_work.is_some_and(|since| since < HOLD) {
        rows.push(row_of(RowKey::Crop, String::new(), &lines.crop, false, rules));
    }
    for lane in &lines.lanes {
        if lane.paused || lane.since_work.is_some_and(|since| since < HOLD) {
            rows.push(row_of(RowKey::Lane(lane.number), lane.label.clone(), &lane.progress, lane.paused, rules));
        }
    }
    rows.sort_by_key(|row| row.key);
    rows.dedup_by_key(|row| row.key);
    rows
}

/// Whether some row exists only because of the hold: it has no work now and no pause of its own. The model is asked
/// again until that row goes.
pub(crate) fn settling(rows: &[LaneRow]) -> bool {
    rows.iter().any(|row| !row.working && !row.paused)
}

/// The change from one list of rows to the next, as the model reports it.
#[derive(Debug, PartialEq)]
pub(crate) struct Plan {
    /// The removes, inserts and moves that make the old keys the new ones, in order (`list_diff::diff`).
    pub(crate) ops: Vec<Op>,
    /// The new list's indices whose key was already there and whose row differs: each is a `dataChanged`.
    pub(crate) changed: Vec<usize>,
    pub(crate) settling: bool,
}

/// The change from `old` to `new`. A row is changed when its key is in both and its content differs; its index is the
/// new list's, since the changes are applied first.
pub(crate) fn plan(old: &[LaneRow], new: &[LaneRow]) -> Plan {
    let old_keys: Vec<RowKey> = old.iter().map(|row| row.key).collect();
    let new_keys: Vec<RowKey> = new.iter().map(|row| row.key).collect();
    let changed = new
        .iter()
        .enumerate()
        .filter(|&(_, row)| old.iter().any(|kept| kept.key == row.key && kept != row))
        .map(|(at, _)| at)
        .collect();
    Plan { ops: list_diff::diff(&old_keys, &new_keys), changed, settling: settling(new) }
}

/// The fixture's lines (`--lane-rows`), parsed: one `number|label|paused|since_work|phase|done|total|elapsed` line per
/// lane and one `crop|since_work|phase|done|total|elapsed` line. `since_work` is seconds, or `-` for never. A `phase` is one
/// of `idle`, `import`, `hashes`, `pairing`, `borders`, `thumbs`, `placeholders`, `crop`.
/// Returns the lines, or the first line that doesn't parse, with its number.
pub(crate) fn parse_fixture(text: &str) -> Result<LaneLines, String> {
    let mut lines = LaneLines { crop: Progress::idle(), crop_since_work: None, lanes: Vec::new() };
    for (at, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let at = at + 1;
        let fields: Vec<&str> = line.split('|').map(str::trim).collect();
        match fields.as_slice() {
            ["crop", since, phase, done, total, elapsed] => {
                lines.crop = progress_of(phase, done, total, elapsed).map_err(|e| format!("line {at}: {e}"))?;
                lines.crop_since_work = since_of(since).map_err(|e| format!("line {at}: {e}"))?;
            }
            [number, label, paused, since, phase, done, total, elapsed] => {
                let number = number.parse::<usize>().map_err(|e| format!("line {at}: the number: {e}"))?;
                let lane = LaneProgress {
                    number,
                    label: (*label).to_string(),
                    progress: progress_of(phase, done, total, elapsed).map_err(|e| format!("line {at}: {e}"))?,
                    since_work: since_of(since).map_err(|e| format!("line {at}: {e}"))?,
                    paused: flag_of(paused).map_err(|e| format!("line {at}: {e}"))?,
                };
                lines.lanes.push(lane);
            }
            _ => return Err(format!("line {at}: not a lane or crop line")),
        }
    }
    Ok(lines)
}

fn progress_of(phase: &str, done: &str, total: &str, elapsed: &str) -> Result<Progress, String> {
    Ok(Progress {
        phase: phase_of(phase).ok_or_else(|| format!("no phase {phase}"))?,
        done: done.parse::<u64>().map_err(|e| format!("done: {e}"))?,
        total: total.parse::<u64>().map_err(|e| format!("total: {e}"))?,
        failed: 0,
        bytes: 0,
        elapsed: seconds_of(elapsed)?,
        since: Mark::default(),
    })
}

fn phase_of(name: &str) -> Option<Phase> {
    Some(match name {
        "idle" => Phase::Idle,
        "import" => Phase::Import,
        "hashes" => Phase::Hashes,
        "pairing" => Phase::Pairing,
        "borders" => Phase::Borders,
        "thumbs" => Phase::Thumbs { height: ThumbHeight::new(256)? },
        "placeholders" => Phase::Placeholders,
        "crop" => Phase::Crop,
        _ => return None,
    })
}

fn seconds_of(text: &str) -> Result<Duration, String> {
    text.parse::<f64>()
        .ok()
        .and_then(|secs| Duration::try_from_secs_f64(secs).ok())
        .ok_or_else(|| format!("{text} is not a time in seconds"))
}

fn since_of(text: &str) -> Result<Option<Duration>, String> {
    if text == "-" { Ok(None) } else { seconds_of(text).map(Some) }
}

fn flag_of(text: &str) -> Result<bool, String> {
    match text {
        "0" => Ok(false),
        "1" => Ok(true),
        _ => Err(format!("{text} is not 0 or 1")),
    }
}

/// The fixture `--lane-rows` names, read once. A file that can't be read or parsed shows no rows, and says why in the log.
fn load_fixture() -> Option<LaneLines> {
    let path = command_line::lane_rows()?;
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) => {
            pg_log::warn("lane model", &format!("the lane rows {} can't be read: {e}", path.display()));
            return Some(LaneLines { crop: Progress::idle(), crop_since_work: None, lanes: Vec::new() });
        }
    };
    match parse_fixture(&text) {
        Ok(fixture) => Some(fixture),
        Err(e) => {
            pg_log::warn("lane model", &format!("the lane rows {} don't parse: {e}", path.display()));
            Some(LaneLines { crop: Progress::idle(), crop_since_work: None, lanes: Vec::new() })
        }
    }
}

fn qt_int(n: usize) -> i32 {
    i32::try_from(n).unwrap_or(i32::MAX)
}

impl qobject::LaneModel {
    pub(crate) fn data(&self, index: &QModelIndex, role: i32) -> QVariant {
        let Some(row) = usize::try_from(index.row()).ok().and_then(|at| self.rows.get(at)) else {
            return QVariant::default();
        };
        match role {
            ROLE_NUMBER => QVariant::from(&row.number),
            ROLE_PAUSABLE => QVariant::from(&row.pausable),
            ROLE_LABEL => QVariant::from(&QString::from(&row.label)),
            ROLE_STATUS => QVariant::from(&QString::from(&row.status)),
            ROLE_PAUSED => QVariant::from(&row.paused),
            ROLE_WORKING => QVariant::from(&row.working),
            ROLE_PROGRESS => QVariant::from(&row.progress),
            ROLE_INDETERMINATE => QVariant::from(&row.indeterminate),
            ROLE_FAILED => QVariant::from(&QString::from(&row.failed)),
            ROLE_FAILED_PASS => QVariant::from(&row.failed_pass),
            _ => QVariant::default(),
        }
    }

    pub(crate) fn role_names(&self) -> QHash<QHashPair_i32_QByteArray> {
        let mut roles = QHash::<QHashPair_i32_QByteArray>::default();
        for (role, name) in [
            (ROLE_NUMBER, "number"),
            (ROLE_PAUSABLE, "pausable"),
            (ROLE_LABEL, "label"),
            (ROLE_STATUS, "status"),
            (ROLE_PAUSED, "paused"),
            (ROLE_WORKING, "working"),
            (ROLE_PROGRESS, "progress"),
            (ROLE_INDETERMINATE, "indeterminate"),
            (ROLE_FAILED, "failed"),
            (ROLE_FAILED_PASS, "failedPass"),
        ] {
            roles.insert(role, QByteArray::from(name));
        }
        roles
    }

    pub(crate) fn row_count(&self, _parent: &QModelIndex) -> i32 {
        qt_int(self.rows.len())
    }

    /// Brings the rows up to the job now (or the fixture): the changes from the rows shown, each row whose content changed
    /// told so, then `settling`. Asked on each progress pass, and by the timer while `settling`.
    pub(crate) fn refresh(mut self: Pin<&mut Self>) {
        if !self.loaded {
            let fixture = load_fixture();
            self.as_mut().rust_mut().fixture = fixture;
            self.as_mut().rust_mut().loaded = true;
        }
        let rules = pg_platform::native::PATH_RULES;
        let new: Vec<LaneRow> = match &self.fixture {
            Some(lines) => rows_of(lines, rules),
            None => match running() {
                Some(library) => rows_of(&library.lane_lines(), rules),
                None => Vec::new(),
            },
        };
        let change = plan(&self.rows, &new);
        let root = QModelIndex::default();
        for &op in &change.ops {
            match op {
                Op::Remove { at, count } => {
                    self.as_mut().begin_remove_rows(&root, qt_int(at), qt_int(at + count - 1));
                    list_diff::apply(&mut self.as_mut().rust_mut().rows, op, &new);
                    self.as_mut().end_remove_rows();
                }
                Op::Insert { at, count } => {
                    self.as_mut().begin_insert_rows(&root, qt_int(at), qt_int(at + count - 1));
                    list_diff::apply(&mut self.as_mut().rust_mut().rows, op, &new);
                    self.as_mut().end_insert_rows();
                }
                Op::Move { from, to } => {
                    // A row moves up only (`to < from`), which Qt always takes.
                    let _ = self.as_mut().begin_move_rows(&root, qt_int(from), qt_int(from), &root, qt_int(to));
                    list_diff::apply(&mut self.as_mut().rust_mut().rows, op, &new);
                    self.as_mut().end_move_rows();
                }
            }
        }
        // The order is now the new one; the kept rows take their new content, which `dataChanged` then reports.
        self.as_mut().rust_mut().rows = new;
        for &at in &change.changed {
            let index = self.index(qt_int(at), 0, &root);
            self.as_mut().data_changed(&index, &index, &QList::default());
        }
        // The fixture is still: it never asks to be refreshed again.
        let settling = change.settling && self.fixture.is_none();
        if self.settling != settling {
            self.as_mut().rust_mut().settling = settling;
            self.as_mut().settling_changed();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pg_model::rules::PathRules as Rules;

    const RULES: Rules = Rules::Windows;

    fn working(done: u64, total: u64) -> Progress {
        Progress { phase: Phase::Import, done, total, ..Progress::idle() }
    }

    fn lane(number: usize, progress: Progress, since_work: Option<Duration>, paused: bool) -> LaneProgress {
        LaneProgress { number, label: format!("{number}:"), progress, since_work, paused }
    }

    fn lines(crop_since: Option<Duration>, lanes: Vec<LaneProgress>) -> LaneLines {
        LaneLines { crop: Progress::idle(), crop_since_work: crop_since, lanes }
    }

    /// A row with the given key and status, working, for the change tests.
    fn row(key: RowKey, status: &str) -> LaneRow {
        LaneRow {
            key,
            number: number_of(key),
            pausable: matches!(key, RowKey::Lane(_)),
            label: String::new(),
            status: status.into(),
            failed: String::new(),
            failed_pass: -1,
            paused: false,
            working: true,
            progress: 0.0,
            indeterminate: false,
        }
    }

    #[test]
    fn a_working_lane_and_a_paused_one_have_rows_and_a_quiet_one_has_none() {
        let working_lane = lane(1, working(1, 4), Some(Duration::ZERO), false);
        let paused_idle = lane(2, Progress::idle(), None, true);
        let quiet = lane(3, Progress::idle(), Some(Duration::from_secs(10)), false);
        let rows = rows_of(&lines(None, vec![working_lane, paused_idle, quiet]), RULES);
        assert_eq!(rows.iter().map(|r| r.key).collect::<Vec<_>>(), vec![RowKey::Lane(1), RowKey::Lane(2)]);
        assert!(rows[0].working && !rows[0].paused);
        assert!(!rows[1].working && rows[1].paused, "the paused idle lane keeps its row, with no bar");
    }

    #[test]
    fn a_lane_in_its_hold_keeps_its_row_with_a_blank_status_until_the_hold_ends() {
        let holding = lane(1, Progress::idle(), Some(Duration::from_secs(1)), false);
        let rows = rows_of(&lines(None, vec![holding]), RULES);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].status, "", "an idle lane's line is empty");
        assert!(!rows[0].working);
        let gone = lane(1, Progress::idle(), Some(HOLD), false);
        assert!(rows_of(&lines(None, vec![gone]), RULES).is_empty(), "at the hold's end the row goes");
    }

    #[test]
    fn the_crop_is_the_first_row_and_is_not_pausable() {
        let crop = Progress { phase: Phase::Crop, done: 2, total: 5, ..Progress::idle() };
        let lines = LaneLines {
            crop,
            crop_since_work: Some(Duration::ZERO),
            lanes: vec![lane(7, working(1, 4), Some(Duration::ZERO), false)],
        };
        let rows = rows_of(&lines, RULES);
        assert_eq!(rows[0].key, RowKey::Crop);
        assert_eq!(rows[0].number, -1);
        assert!(!rows[0].pausable);
        assert_eq!(rows[1].key, RowKey::Lane(7));
        assert!(rows[1].pausable);
    }

    #[test]
    fn the_status_has_no_label_and_the_paused_wording_for_a_paused_lane() {
        let running = lines(None, vec![lane(1, working(1, 4), Some(Duration::ZERO), false)]);
        let rows = rows_of(&running, RULES);
        assert!(!rows[0].status.contains("1:"), "the drive's label is the row's own");
        assert_eq!(rows[0].status, text::pass_status(&working(1, 4), RULES));
        let paused = lines(None, vec![lane(1, working(1, 4), Some(Duration::ZERO), true)]);
        assert_eq!(rows_of(&paused, RULES)[0].status, text::lane_paused_status(&working(1, 4), RULES));
    }

    #[test]
    fn an_unknown_total_is_indeterminate_with_no_progress_and_a_known_one_is_not() {
        let unknown = lane(1, working(3, 0), Some(Duration::ZERO), false);
        let known = lane(2, working(1, 4), Some(Duration::ZERO), false);
        let rows = rows_of(&lines(None, vec![unknown, known]), RULES);
        assert!(rows[0].indeterminate);
        assert_eq!(rows[0].progress, 0.0);
        assert!(!rows[1].indeterminate);
        assert_eq!(rows[1].progress, 0.25);
    }

    #[test]
    fn a_duplicate_key_is_dropped_and_the_order_is_the_crop_then_the_lane_numbers() {
        let a = lane(3, working(1, 4), Some(Duration::ZERO), false);
        let b = lane(1, working(1, 4), Some(Duration::ZERO), false);
        let again = lane(3, working(2, 4), Some(Duration::ZERO), false);
        let lines =
            LaneLines { crop: Progress::idle(), crop_since_work: Some(Duration::ZERO), lanes: vec![a, b, again] };
        let keys: Vec<RowKey> = rows_of(&lines, RULES).iter().map(|r| r.key).collect();
        assert_eq!(keys, vec![RowKey::Crop, RowKey::Lane(1), RowKey::Lane(3)]);
    }

    #[test]
    fn settling_is_for_a_row_that_exists_by_the_hold_alone() {
        assert!(!settling(&[row(RowKey::Lane(1), "")]), "a working row is not held");
        let mut held = row(RowKey::Lane(1), "");
        held.working = false;
        assert!(settling(&[held.clone()]));
        held.paused = true;
        assert!(!settling(&[held]), "a paused row is not waiting on the hold");
    }

    #[test]
    fn a_status_only_change_is_one_changed_row_and_no_change_of_order() {
        let old = vec![row(RowKey::Crop, "1"), row(RowKey::Lane(1), "a")];
        let new = vec![row(RowKey::Crop, "1"), row(RowKey::Lane(1), "b")];
        let plan = plan(&old, &new);
        assert!(plan.ops.is_empty());
        assert_eq!(plan.changed, vec![1]);
    }

    #[test]
    fn a_lane_appearing_is_one_insert_and_nothing_changed() {
        let old = vec![row(RowKey::Lane(1), "a")];
        let new = vec![row(RowKey::Lane(0), "z"), row(RowKey::Lane(1), "a")];
        let plan = plan(&old, &new);
        assert_eq!(plan.ops, vec![Op::Insert { at: 0, count: 1 }]);
        assert!(plan.changed.is_empty(), "the kept row is unchanged, so it is not told");
    }

    #[test]
    fn the_first_row_removed_and_the_second_changed_reports_the_new_index_of_the_changed_one() {
        let old = vec![row(RowKey::Lane(1), "a"), row(RowKey::Lane(2), "b")];
        let new = vec![row(RowKey::Lane(2), "c")];
        let plan = plan(&old, &new);
        assert_eq!(plan.ops, vec![Op::Remove { at: 0, count: 1 }]);
        assert_eq!(plan.changed, vec![0]);
    }

    #[test]
    fn a_row_carries_its_passes_failed_count_and_the_ledger_it_is_listed_under() {
        let import = Progress { phase: Phase::Import, done: 1, total: 4, failed: 3, ..Progress::idle() };
        let rows = rows_of(&lines(None, vec![lane(1, import.clone(), Some(Duration::ZERO), false)]), RULES);
        assert_eq!(rows[0].failed, text::pass_failed(&import));
        assert!(!rows[0].failed.is_empty());
        assert_eq!(rows[0].failed_pass, kind_code(Some(LedgerKind::File(FilePass::Import))));
        let crop = Progress { phase: Phase::Crop, done: 1, total: 4, failed: 2, ..Progress::idle() };
        let rows = rows_of(&LaneLines { crop, crop_since_work: Some(Duration::ZERO), lanes: vec![] }, RULES);
        assert_eq!(rows[0].failed_pass, -1, "the crop lists its failures under no ledger");
    }

    #[test]
    fn a_fixture_parses_its_lanes_and_crop() {
        let text = "1|E:|0|0|import|10|40|5\r\n2|D:|1|-|hashes|3|0|2\r\ncrop|-|idle|0|0|0\r\n";
        let lines = parse_fixture(text).expect("the fixture parses");
        assert_eq!(lines.lanes.len(), 2);
        assert_eq!(lines.lanes[0].label, "E:");
        assert_eq!(lines.lanes[0].since_work, Some(Duration::ZERO));
        assert_eq!(lines.lanes[0].progress.total, 40);
        assert!(lines.lanes[1].paused);
        assert_eq!(lines.lanes[1].since_work, None);
        assert_eq!(lines.crop_since_work, None);
    }

    #[test]
    fn a_fixture_line_that_does_not_parse_is_an_error_with_its_number() {
        let error = |text: &str| parse_fixture(text).err().unwrap_or_default();
        assert!(error("1|E:|0|0|nosuchphase|1|2|3").contains("line 1"));
        assert!(error("\nnot|a|lane").contains("line 2"));
        assert!(error("1|E:").contains("line 1"));
    }
}
