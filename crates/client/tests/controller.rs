//! The sub-machines of `pg_client::controller`, by traces: each case is events in an order the desktop can report them,
//! and the commands they must give.

#[path = "controller/deadlines.rs"]
mod deadlines;
#[path = "controller/input.rs"]
mod input;
#[path = "controller/listing.rs"]
mod listing;
#[path = "controller/panels.rs"]
mod panels;
#[path = "controller/prefs.rs"]
mod prefs;
#[path = "controller/start.rs"]
mod start;
