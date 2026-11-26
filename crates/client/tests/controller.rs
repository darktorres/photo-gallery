//! The sub-machines of `pg_client::controller`, by traces: each case is events in an order the desktop can report them,
//! and the commands they must give.

#[path = "common/mod.rs"]
mod common;
#[path = "controller/deadlines.rs"]
mod deadlines;
#[path = "controller/input.rs"]
mod input;
#[path = "controller/listing.rs"]
mod listing;
#[path = "controller/panels.rs"]
mod panels;
#[path = "controller/size.rs"]
mod size;
#[path = "controller/start.rs"]
mod start;
