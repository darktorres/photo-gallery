//! Replays a recorded session (`tests/replay/session.jsonl`, written by the desktop with `PG_RECORD`): the controller starts
//! as the window did, each recorded event is applied at its recorded time, and every command it gives must be the one
//! recorded. The recording is an offscreen run of `gallery.exe` with `--apply-view=alpha --filters=1 --view=0
//! --viewer-keys=r3`: the start, the layout, the filter bar, a rating in the viewer, and the pauses.

use pg_client::controller::Command;
use pg_client::controller::replay::{Record, parse, replay};

/// The recording's records; a recording that does not parse (or is of another format) fails the test with its reason.
fn recording() -> Result<Vec<Record>, String> {
    parse(include_str!("replay/session.jsonl"))
}

#[test]
fn the_recorded_session_replays_to_the_recorded_commands() -> Result<(), String> {
    let records = recording()?;
    assert_eq!(replay(&records), Ok(records.len() - 1));
    Ok(())
}

#[test]
fn the_recording_contains_the_start_and_a_model_command() -> Result<(), String> {
    let records = recording()?;
    let gave = |want: fn(&Command) -> bool| {
        records.iter().any(|r| matches!(r, Record::Update { commands, .. } if commands.iter().any(want)))
    };
    assert!(gave(|c| matches!(c, Command::Started)), "the recording reaches the start");
    assert!(gave(|c| matches!(c, Command::Rate { .. })), "the recording rates an item");
    Ok(())
}

#[test]
fn a_changed_command_is_reported_at_its_record() -> Result<(), String> {
    let mut records = recording()?;
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
    Ok(())
}

#[test]
fn a_recording_of_another_format_is_refused_by_its_version() {
    // A start written before the format was recorded: its Init has no `format`, so it is format 1.
    let old = r#"{"kind":"Init","listing":{},"row_height":220,"saved_row":220}"#;
    let error = parse(old).expect_err("a format-1 recording must be refused");
    assert_eq!(error, format!("recorded by format 1, this build reads {}", pg_client::controller::replay::FORMAT));
}
