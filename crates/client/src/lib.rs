//! The front end's logic: what the desktop and the web client both need, with no Qt and no platform calls.

#![forbid(unsafe_code)]

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
pub mod text;
pub mod tree;
pub mod viewport;
