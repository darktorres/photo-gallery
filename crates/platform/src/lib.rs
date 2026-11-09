//! The operating system behind portable functions. Win32 calls are made here, and only here: each function has the
//! Windows behaviour the app relies on, and a portable one elsewhere, so the library crates above it build on both.

// A library reports through its results; printing is for the command line and the app.
#![cfg_attr(not(test), deny(clippy::print_stdout, clippy::print_stderr))]

pub mod clock;
pub mod dir;
pub mod dirs;
pub mod file;
pub mod message;
pub mod names;
pub mod native;
pub mod picker;
pub mod priority;
pub mod process;
pub mod sqlite;
pub mod volume;
