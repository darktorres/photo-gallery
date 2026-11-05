//! The front end's policy as sub-machines: each takes the facts the desktop reports as events and says what to do as
//! commands, with no Qt and no clock. The desktop's `Shell` holds them and carries the commands out (desktop/shell.rs).
//! Time, when a sub-machine needs it, is an input: a `now_ms` passed with each event.

pub mod deadlines;
pub mod input;
pub mod listing;
pub mod panels;
pub mod prefs;
pub mod start;
