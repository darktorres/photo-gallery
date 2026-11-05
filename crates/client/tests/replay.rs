//! Replays a recorded session (`tests/replay/session.jsonl`, written by the desktop with `PG_RECORD`): the controller starts
//! as the window did, each recorded event is applied at its recorded time, and every command it gives must be the one
//! recorded. The recording is an offscreen run of `gallery.exe` with `--apply-view=alpha --filters=1 --view=0
//! --viewer-keys=r3`: the start, the layout, the filter bar, a rating in the viewer, and the pauses.

use pg_client::controller::Command;
use pg_client::controller::replay::{Record, replay};

fn recording() -> Vec<Record> {
    include_str!("replay/session.jsonl")
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("a record of the recording"))
        .collect()
}

#[test]
fn the_recorded_session_replays_to_the_recorded_commands() {
    let records = recording();
    assert_eq!(replay(&records), Ok(records.len() - 1));
}

#[test]
fn the_recording_contains_the_start_and_a_model_command() {
    let records = recording();
    let gave = |want: fn(&Command) -> bool| {
        records.iter().any(|r| matches!(r, Record::Update { commands, .. } if commands.iter().any(want)))
    };
    assert!(gave(|c| matches!(c, Command::Started)), "the recording reaches the start");
    assert!(gave(|c| matches!(c, Command::Rate { .. })), "the recording rates an item");
}

#[test]
fn a_changed_command_is_reported_at_its_record() {
    let mut records = recording();
    let index = records
        .iter()
        .position(
            |r| matches!(r, Record::Update { commands, .. } if commands.iter().any(|c| matches!(c, Command::Started))),
        )
        .expect("the recording starts");
    if let Record::Update { commands, .. } = &mut records[index] {
        commands.clear();
    }
    let error = replay(&records).expect_err("a changed command must not replay");
    assert!(error.starts_with(&format!("record {}:", index)), "{error}");
}
