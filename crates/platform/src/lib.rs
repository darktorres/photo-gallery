//! The operating system behind portable functions. Win32 calls are made here, and only here: each function has the
//! Windows behaviour the app relies on, and a portable one elsewhere, so the library crates above it build on both.

pub mod args;
pub mod audio;
pub mod clock;
pub mod console;
pub mod dir;
pub mod dirs;
pub mod dll;
pub mod drive;
pub mod file;
pub mod memory;
pub mod message;
pub mod names;
pub mod native;
pub mod picker;
pub mod priority;
pub mod process;
pub mod sqlite;
#[cfg(any(test, feature = "testing"))]
pub mod testing;
pub mod volume;
