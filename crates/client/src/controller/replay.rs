//! A recorded session, and its replay (S-12.18). The desktop writes one JSON line per record when `PG_RECORD` names a
//! file: first `Init` (the start the controller was made from), then one `Update` per event, with the time it was
//! applied at and the commands it gave. `replay` starts a controller from `Init`, applies
//! each `Update`'s event at its time, and checks that the commands are the recorded ones. Time is an input, so nothing
//! here reads a clock.

use serde::{Deserialize, Serialize};

use super::{Command, Controller, Event, Start};

/// The recording's format: bumped whenever a record's shape changes, so a file of another format is refused with its
/// version (`parse`), not with a parse error. A file with no `format` on its `Init` is format 1.
pub const FORMAT: u32 = 3;

/// One line of a recording.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum Record {
    /// The controller's start: the format it is written in, and the start it was made from. The window's scale comes as
    /// the first size event, as it does live.
    Init { format: u32, start: Start },
    /// An event, applied at `now_ms`, and the commands it gave.
    Update { now_ms: u64, event: Event, commands: Vec<Command> },
}

/// A record for the controller's start, as `Controller::new` takes it.
pub fn init(controller: &Controller) -> Record {
    Record::Init { format: FORMAT, start: controller.initial().clone() }
}

/// The records of a recording's text, one per line. A recording of another format is refused by its version, before its
/// records are read as this format's.
pub fn parse(text: &str) -> Result<Vec<Record>, String> {
    /// The part of a line that tells its format: an `Init` with no `format` is format 1.
    #[derive(Deserialize)]
    struct Head {
        kind: String,
        #[serde(default)]
        format: Option<u32>,
    }
    let mut records = Vec::new();
    for (n, line) in text.lines().enumerate().filter(|(_, line)| !line.trim().is_empty()) {
        let head: Head = serde_json::from_str(line).map_err(|e| format!("line {}: {e}", n + 1))?;
        if head.kind == "Init" {
            let format = head.format.unwrap_or(1);
            if format != FORMAT {
                return Err(format!("recorded by format {format}, this build reads {FORMAT}"));
            }
        }
        records.push(serde_json::from_str(line).map_err(|e| format!("line {}: {e}", n + 1))?);
    }
    Ok(records)
}

/// Replay a recording: the number of events checked, or the first record whose commands differ from the recorded ones.
pub fn replay(records: &[Record]) -> Result<usize, String> {
    let Some((first, rest)) = records.split_first() else {
        return Err("an empty recording".to_string());
    };
    let Record::Init { format, start } = first else {
        return Err("a recording starts with its Init record".to_string());
    };
    if *format != FORMAT {
        return Err(format!("recorded by format {format}, this build reads {FORMAT}"));
    }
    let mut controller = Controller::new(start.clone());
    for (n, record) in rest.iter().enumerate() {
        let Record::Update { now_ms, event, commands } = record else {
            return Err(format!("record {}: a second Init", n + 1));
        };
        let got = controller.update(*now_ms, event.clone());
        if &got != commands {
            return Err(format!("record {}: the commands {got:?}, recorded {commands:?}", n + 1));
        }
    }
    Ok(rest.len())
}
