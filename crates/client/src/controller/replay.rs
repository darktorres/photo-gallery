//! A recorded session, and its replay (S-12.18). The desktop writes one JSON line per record when `PG_RECORD` names a
//! file: first `Init` (the listing, the row height and the scale the controller started with), then one `Update` per
//! event, with the time it was applied at and the commands it gave. `replay` starts a controller from `Init`, applies
//! each `Update`'s event at its time, and checks that the commands are the recorded ones. Time is an input, so nothing
//! here reads a clock.

use pg_model::view::Listing;
use serde::{Deserialize, Serialize};

use super::{Command, Controller, Event};

/// One line of a recording.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum Record {
    /// The controller's start: its listing, row height and scale.
    Init { listing: Listing, row_height: i32, scale: f64 },
    /// An event, applied at `now_ms`, and the commands it gave.
    Update { now_ms: u64, event: Event, commands: Vec<Command> },
}

/// A record for the controller's start, as `Controller::new` takes it.
pub fn init(controller: &Controller) -> Record {
    Record::Init {
        listing: controller.listing().clone(),
        row_height: controller.prefs().row_height(),
        scale: controller.prefs().scale(),
    }
}

/// Replay a recording: the number of events checked, or the first record whose commands differ from the recorded ones.
pub fn replay(records: &[Record]) -> Result<usize, String> {
    let Some((first, rest)) = records.split_first() else {
        return Err("an empty recording".to_string());
    };
    let Record::Init { listing, row_height, scale } = first else {
        return Err("a recording starts with its Init record".to_string());
    };
    let mut controller = Controller::new(listing.clone(), *row_height, *scale);
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
