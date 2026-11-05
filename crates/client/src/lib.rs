//! The front end's logic: what the desktop and the web client both need, with no Qt and no platform calls (PLAN §7).

#![cfg_attr(not(test), deny(clippy::print_stdout, clippy::print_stderr))]

pub mod controller;
pub mod facets;
pub mod grid;
pub mod input;
pub mod keys;
pub mod layout;
pub mod picks;
pub mod sort;
pub mod text;
pub mod tree;
pub mod viewport;
