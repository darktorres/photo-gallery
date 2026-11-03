//! The job layer: the parallel pipeline the passes over the originals run on, and the control a running pass
//! shares with whoever started it (progress, pause, stop). Nothing here knows what a pass does to a file.

pub mod control;
pub mod pipeline;
