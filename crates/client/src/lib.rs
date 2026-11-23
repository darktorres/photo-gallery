//! The front end's logic: what the desktop and the web client both need, with no Qt and no platform calls.

// no_std: the logic reads no clock, thread, file, socket, process or environment, by construction. Its words are
// `pg-wording`'s (a std crate above this one), so nothing here needs std but the tests.
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;
#[cfg(test)]
extern crate std;

pub mod controller;
pub mod facets;
pub mod features;
pub mod grid;
pub mod image;
pub mod input;
pub mod keys;
pub mod layout;
pub mod picks;
pub mod results;
pub mod sort;
pub mod tree;
pub mod viewport;
