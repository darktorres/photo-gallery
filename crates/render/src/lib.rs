//! The display images (PLAN §6): the pixels the app shows or sends, made from the files. [`maker`] makes the grid's
//! thumbnails and the viewer's images in the thread's own buffers; [`display`] decodes a file once, upright, at the
//! smallest size a caller needs. Both read the EXIF orientation here and hand it to the codec, which reads none.
//! [`player`] plays an animation's frames into a [`player::Sink`] at their delays.

// A library reports through its results; printing is for the command line and the app.
#![cfg_attr(not(test), deny(clippy::print_stdout, clippy::print_stderr))]

pub mod display;
pub mod maker;
pub mod player;
